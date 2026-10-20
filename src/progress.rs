use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

pub const ORANGE: &str = "\x1b[38;2;255;140;0m";
pub const SLATE_CYAN: &str = "\x1b[38;2;90;145;165m";
pub const GREEN: &str = SLATE_CYAN;
pub const DIM_GREY: &str = "\x1b[38;2;90;90;90m";
pub const RESET: &str = "\x1b[0m";

pub const DOWNLOAD_INDENT: &str = "  \x1b[38;2;90;90;90m│\x1b[0m  ";
pub const INSTALL_INDENT: &str = "  \x1b[38;2;90;90;90m│\x1b[0m  ";
pub const TREE_TRUNK_INDENT: &str = "  \x1b[38;2;90;90;90m│\x1b[0m  ";
pub const TREE_LAST_INDENT: &str = "     ";

#[derive(Clone, Debug)]
pub enum TranscriptEntry {
    Text {
        line: String,
        rendered_width: usize,
        term_width: usize,
    },
    Bar {
        bytes: u64,
        total: Option<u64>,
        speed: f64,
        elapsed: Duration,
        bar_color: &'static str,
        indent: &'static str,
        rendered_width: usize,
        term_width: usize,
    },
}

/// Tracks all lines and completed progress bars in the current multi-package tree
/// so that if the user resizes the terminal window at any point during downloading or installing,
/// all previously completed bars in the tree proactively resize to match the active bar.
#[derive(Default, Clone, Debug)]
pub struct TreeTranscript {
    pub entries: Vec<TranscriptEntry>,
}

impl TreeTranscript {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Prints a line to stdout and records it in the tree transcript for resize redraws.
    pub fn println(&mut self, line: impl Into<String>) {
        let s = line.into();
        let term_width = get_terminal_width();
        let rendered_width = visible_width(&s);
        println!("{}", s);
        let _ = io::stdout().flush();
        self.entries.push(TranscriptEntry::Text {
            line: s,
            rendered_width,
            term_width,
        });
    }

    /// Records a completed progress bar in the transcript.
    pub fn push_completed_bar(
        &mut self,
        bytes: u64,
        total: Option<u64>,
        elapsed: Duration,
        bar_color: &'static str,
        indent: &'static str,
        rendered_width: usize,
        term_width: usize,
    ) {
        let speed = if elapsed.as_secs_f64() > 0.0 {
            bytes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        self.entries.push(TranscriptEntry::Bar {
            bytes,
            total,
            speed,
            elapsed,
            bar_color,
            indent,
            rendered_width,
            term_width,
        });
    }

    /// Redraws the entire recorded tree plus the `active_line` at `new_term_width`,
    /// clearing any soft-wrapped ghost lines produced when shrinking the terminal window.
    pub fn redraw_all_with_active(
        &mut self,
        active_line: &str,
        active_prev_rendered_width: usize,
        active_prev_term_width: usize,
        new_term_width: usize,
    ) {
        let active_extra = if active_prev_rendered_width > 0 && new_term_width < active_prev_term_width {
            active_prev_rendered_width.saturating_sub(1) / new_term_width
        } else {
            0
        };

        let mut total_up = active_extra;
        for entry in &self.entries {
            let (w, old_tw) = match entry {
                TranscriptEntry::Text {
                    rendered_width,
                    term_width,
                    ..
                } => (*rendered_width, *term_width),
                TranscriptEntry::Bar {
                    rendered_width,
                    term_width,
                    ..
                } => (*rendered_width, *term_width),
            };
            let extra = if w > 0 && new_term_width < old_tw {
                w.saturating_sub(1) / new_term_width
            } else {
                0
            };
            total_up += 1 + extra;
        }

        clear_wrapped_rows(total_up);

        for entry in &mut self.entries {
            match entry {
                TranscriptEntry::Text {
                    line,
                    rendered_width,
                    term_width,
                } => {
                    print!("\r\x1b[K{}\n", line);
                    *rendered_width = visible_width(line);
                    *term_width = new_term_width;
                }
                TranscriptEntry::Bar {
                    bytes,
                    total,
                    speed,
                    elapsed,
                    bar_color,
                    indent,
                    rendered_width,
                    term_width,
                } => {
                    let mut marquee = 0;
                    let re_rendered = render_colored_progress_line(
                        *bytes,
                        *total,
                        *speed,
                        *elapsed,
                        true,
                        &mut marquee,
                        new_term_width,
                        bar_color,
                        indent,
                    );
                    print!("\r\x1b[K{}\n", re_rendered);
                    *rendered_width = visible_width(&re_rendered);
                    *term_width = new_term_width;
                }
            }
        }

        print!("\r\x1b[K{}", active_line);
        let _ = io::stdout().flush();
    }
}

/// Summary of a completed download bar so a subsequent `InstallProgress` bar
/// can proactively redraw the download bar if the user resizes the terminal window.
#[derive(Debug, Clone, Copy)]
pub struct DownloadSummary {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub elapsed: Duration,
    pub rendered_width: usize,
    pub term_width: usize,
}

/// Clears `extra_rows` of soft-wrapped terminal output above the current cursor line.
fn clear_wrapped_rows(extra_rows: usize) {
    print!("\r\x1b[K");
    for _ in 0..extra_rows {
        print!("\x1b[1A\r\x1b[K");
    }
}

/// Download progress bar with pip-like styling:
/// Classic block style: Solid orange blocks with a dim grey shaded background,
/// percentage on the left, downloaded/total MB, speed, and ETA on the right.
/// Dynamically scales to terminal width and stays on a single line.
pub struct DownloadProgress<'a> {
    total_bytes: Option<u64>,
    downloaded_bytes: u64,
    start_time: Instant,
    last_render_time: Instant,
    last_render_bytes: u64,
    smoothed_speed: f64,
    is_terminal: bool,
    last_marquee_pos: usize,
    last_term_width: usize,
    last_rendered_width: usize,
    indent: &'static str,
    transcript: Option<&'a mut TreeTranscript>,
}

impl<'a> DownloadProgress<'a> {
    pub fn new(total_bytes: Option<u64>) -> Self {
        Self::with_tree(total_bytes, DOWNLOAD_INDENT, None)
    }

    pub fn with_tree(
        total_bytes: Option<u64>,
        indent: &'static str,
        transcript: Option<&'a mut TreeTranscript>,
    ) -> Self {
        let is_terminal = io::stdout().is_terminal();
        let now = Instant::now();
        let term_width = get_terminal_width();
        let mut progress = Self {
            total_bytes,
            downloaded_bytes: 0,
            start_time: now,
            last_render_time: now,
            last_render_bytes: 0,
            smoothed_speed: 0.0,
            is_terminal,
            last_marquee_pos: 0,
            last_term_width: term_width,
            last_rendered_width: 0,
            indent,
            transcript,
        };

        if progress.is_terminal {
            progress.render(false);
        }
        progress
    }

    /// Record newly downloaded bytes and refresh the progress display if throttled interval has elapsed.
    pub fn update(&mut self, chunk_len: usize) {
        self.downloaded_bytes += chunk_len as u64;
        let now = Instant::now();
        let delta = now.duration_since(self.last_render_time);

        // Throttle renders to at most once every 50ms (20 fps) to avoid wasting CPU on terminal I/O
        if delta >= Duration::from_millis(50) {
            let delta_secs = delta.as_secs_f64();
            let delta_bytes = (self.downloaded_bytes - self.last_render_bytes) as f64;
            let instant_speed = if delta_secs > 0.0 {
                delta_bytes / delta_secs
            } else {
                0.0
            };

            // Smooth speed with Exponential Moving Average (EMA)
            self.smoothed_speed = if self.smoothed_speed == 0.0 {
                instant_speed
            } else {
                0.7 * self.smoothed_speed + 0.3 * instant_speed
            };

            self.last_render_time = now;
            self.last_render_bytes = self.downloaded_bytes;

            if self.is_terminal {
                self.render(false);
            }
        }
    }

    /// Complete the progress display with 100% full orange bar and summary metrics,
    /// returning a `DownloadSummary` and recording in `TreeTranscript` if present.
    pub fn finish(&mut self) -> DownloadSummary {
        let elapsed = self.start_time.elapsed();
        if self.is_terminal {
            self.render(true);
            println!();
            let _ = io::stdout().flush();
        } else {
            let elapsed_secs = elapsed.as_secs_f64().max(0.001);
            let mb = self.downloaded_bytes as f64 / 1_048_576.0;
            println!(
                "{}✓ Downloaded {:.1} MB in {:.1}s",
                self.indent, mb, elapsed_secs
            );
        }
        if let Some(ref mut tr) = self.transcript {
            tr.push_completed_bar(
                self.downloaded_bytes,
                self.total_bytes,
                elapsed,
                ORANGE,
                self.indent,
                self.last_rendered_width,
                self.last_term_width,
            );
        }
        DownloadSummary {
            downloaded_bytes: self.downloaded_bytes,
            total_bytes: self.total_bytes,
            elapsed,
            rendered_width: self.last_rendered_width,
            term_width: self.last_term_width,
        }
    }

    fn render(&mut self, is_done: bool) {
        let term_width = get_terminal_width();
        let rendered = render_colored_progress_line(
            self.downloaded_bytes,
            self.total_bytes,
            self.smoothed_speed,
            self.start_time.elapsed(),
            is_done,
            &mut self.last_marquee_pos,
            term_width,
            ORANGE,
            self.indent,
        );

        if term_width != self.last_term_width {
            if let Some(ref mut tr) = self.transcript {
                tr.redraw_all_with_active(
                    &rendered,
                    self.last_rendered_width,
                    self.last_term_width,
                    term_width,
                );
                self.last_rendered_width = visible_width(&rendered);
                self.last_term_width = term_width;
                return;
            }
        }

        if self.last_rendered_width > 0 && term_width < self.last_term_width {
            let extra_rows = self.last_rendered_width.saturating_sub(1) / term_width;
            if extra_rows > 0 {
                clear_wrapped_rows(extra_rows);
            }
        }

        self.last_rendered_width = visible_width(&rendered);
        self.last_term_width = term_width;

        // Carriage return \r + clear line \x1b[K guarantees no artifacts on terminal resize
        print!("\r\x1b[K{}", rendered);
        let _ = io::stdout().flush();
    }
}

/// Installation progress bar with pip-like styling:
/// Matches the exact layout, dynamic terminal width scaling, speed, and ETA metrics
/// of `DownloadProgress`, but styled in slate cyan (`SLATE_CYAN`) to visually distinguish
/// archive extraction and `.app` bundle installation from downloading.
pub struct InstallProgress<'a> {
    total_bytes: Option<u64>,
    installed_bytes: u64,
    start_time: Instant,
    last_render_time: Instant,
    last_render_bytes: u64,
    smoothed_speed: f64,
    is_terminal: bool,
    last_marquee_pos: usize,
    last_term_width: usize,
    last_rendered_width: usize,
    download_summary: Option<DownloadSummary>,
    can_redraw_download_bar: bool,
    indent: &'static str,
    transcript: Option<&'a mut TreeTranscript>,
}

impl<'a> InstallProgress<'a> {
    pub fn new(total_bytes: Option<u64>, download_summary: Option<DownloadSummary>) -> Self {
        let is_terminal = io::stdout().is_terminal();
        let now = Instant::now();
        let term_width = get_terminal_width();
        let mut progress = Self {
            total_bytes,
            installed_bytes: 0,
            start_time: now,
            last_render_time: now,
            last_render_bytes: 0,
            smoothed_speed: 0.0,
            is_terminal,
            last_marquee_pos: 0,
            last_term_width: term_width,
            last_rendered_width: 0,
            download_summary,
            can_redraw_download_bar: download_summary.is_some(),
            indent: INSTALL_INDENT,
            transcript: None,
        };

        if progress.is_terminal {
            progress.render(false);
        }
        progress
    }

    pub fn with_tree(
        total_bytes: Option<u64>,
        indent: &'static str,
        transcript: Option<&'a mut TreeTranscript>,
    ) -> Self {
        let is_terminal = io::stdout().is_terminal();
        let now = Instant::now();
        let term_width = get_terminal_width();
        let mut progress = Self {
            total_bytes,
            installed_bytes: 0,
            start_time: now,
            last_render_time: now,
            last_render_bytes: 0,
            smoothed_speed: 0.0,
            is_terminal,
            last_marquee_pos: 0,
            last_term_width: term_width,
            last_rendered_width: 0,
            download_summary: None,
            can_redraw_download_bar: false,
            indent,
            transcript,
        };

        if progress.is_terminal {
            progress.render(false);
        }
        progress
    }

    /// Sets or updates the known total uncompressed byte size once discovered (e.g. after mounting a DMG).
    pub fn set_total(&mut self, total_bytes: u64) {
        if total_bytes > 0 {
            self.total_bytes = Some(total_bytes);
            if self.is_terminal {
                self.render(false);
            }
        }
    }

    /// Returns the currently configured total bytes, if known.
    pub fn total_bytes(&self) -> Option<u64> {
        self.total_bytes
    }

    /// Updates cumulative installed bytes (e.g. from polling extracted directory size)
    /// and refreshes the terminal bar if the 50ms throttle interval has elapsed.
    pub fn set_installed_bytes(&mut self, bytes: u64) {
        self.installed_bytes = if let Some(tot) = self.total_bytes {
            // Cap in-flight progress at 99% of total so 100% is reserved for finish()
            bytes.min(tot.saturating_sub(1))
        } else {
            bytes
        };

        let now = Instant::now();
        let delta = now.duration_since(self.last_render_time);

        if delta >= Duration::from_millis(50) {
            let delta_secs = delta.as_secs_f64();
            let delta_bytes = self.installed_bytes.saturating_sub(self.last_render_bytes) as f64;
            let instant_speed = if delta_secs > 0.0 {
                delta_bytes / delta_secs
            } else {
                0.0
            };

            if delta_bytes > 0.0 {
                self.smoothed_speed = if self.smoothed_speed == 0.0 {
                    instant_speed
                } else {
                    0.7 * self.smoothed_speed + 0.3 * instant_speed
                };
            }

            self.last_render_time = now;
            self.last_render_bytes = self.installed_bytes;

            if self.is_terminal {
                self.render(false);
            }
        }
    }

    /// Advances the indeterminate marquee animation (e.g. while attaching a DMG) if 50ms has elapsed.
    pub fn tick(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_render_time) >= Duration::from_millis(50) {
            self.last_render_time = now;
            if self.is_terminal {
                self.render(false);
            }
        }
    }

    /// Prints a diagnostic or warning message cleanly above the active progress bar without corrupting the line.
    pub fn println(&mut self, msg: &str) {
        if self.is_terminal {
            self.can_redraw_download_bar = false;
            let term_width = get_terminal_width();
            if self.last_rendered_width > 0 && term_width < self.last_term_width {
                let extra_rows = self.last_rendered_width.saturating_sub(1) / term_width;
                if extra_rows > 0 {
                    clear_wrapped_rows(extra_rows);
                }
            }
            print!("\r\x1b[K{}\n", msg);
            if let Some(ref mut tr) = self.transcript {
                tr.entries.push(TranscriptEntry::Text {
                    line: msg.to_string(),
                    rendered_width: visible_width(msg),
                    term_width,
                });
            }
            self.render(false);
        } else {
            println!("{}", msg);
        }
    }

    /// Completes the installation progress display with a 100% full bar and summary metrics.
    pub fn finish(&mut self, final_total_bytes: Option<u64>) {
        if let Some(final_tot) = final_total_bytes.filter(|&b| b > 0) {
            self.total_bytes = Some(final_tot);
            self.installed_bytes = final_tot;
        } else if let Some(tot) = self.total_bytes {
            self.installed_bytes = tot;
        } else if self.installed_bytes > 0 {
            self.total_bytes = Some(self.installed_bytes);
        }

        let elapsed = self.start_time.elapsed();
        if self.is_terminal {
            self.render(true);
            println!();
            let _ = io::stdout().flush();
        } else {
            let elapsed_secs = elapsed.as_secs_f64().max(0.001);
            let mb = self.installed_bytes as f64 / 1_048_576.0;
            println!(
                "{}✓ Installed {:.1} MB in {:.1}s",
                self.indent, mb, elapsed_secs
            );
        }

        if let Some(ref mut tr) = self.transcript {
            tr.push_completed_bar(
                self.installed_bytes,
                self.total_bytes,
                elapsed,
                GREEN,
                self.indent,
                self.last_rendered_width,
                self.last_term_width,
            );
        }
    }

    /// Clears the in-progress bar line on error so error messages print cleanly.
    pub fn clear(&self) {
        if self.is_terminal {
            print!("\r\x1b[K");
            let _ = io::stdout().flush();
        }
    }

    fn render(&mut self, is_done: bool) {
        let term_width = get_terminal_width();
        let install_rendered = render_colored_progress_line(
            self.installed_bytes,
            self.total_bytes,
            self.smoothed_speed,
            self.start_time.elapsed(),
            is_done,
            &mut self.last_marquee_pos,
            term_width,
            GREEN,
            self.indent,
        );

        if term_width != self.last_term_width {
            if let Some(ref mut tr) = self.transcript {
                tr.redraw_all_with_active(
                    &install_rendered,
                    self.last_rendered_width,
                    self.last_term_width,
                    term_width,
                );
                self.last_rendered_width = visible_width(&install_rendered);
                self.last_term_width = term_width;
                return;
            }
        }

        if self.can_redraw_download_bar && term_width != self.last_term_width {
            if let Some(mut dl) = self.download_summary {
                let install_extra = if self.last_rendered_width > 0 && term_width < self.last_term_width {
                    self.last_rendered_width.saturating_sub(1) / term_width
                } else {
                    0
                };
                let dl_extra = if dl.rendered_width > 0 && term_width < dl.term_width {
                    dl.rendered_width.saturating_sub(1) / term_width
                } else {
                    0
                };

                // Clear current install line, spacer line `│`, and download line (including any soft-wrapped rows)
                let total_up = install_extra + dl_extra + 2;
                clear_wrapped_rows(total_up);

                let mut dl_marquee = 0;
                let dl_speed = if dl.elapsed.as_secs_f64() > 0.0 {
                    dl.downloaded_bytes as f64 / dl.elapsed.as_secs_f64()
                } else {
                    0.0
                };
                let dl_rendered = render_progress_line(
                    dl.downloaded_bytes,
                    dl.total_bytes,
                    dl_speed,
                    dl.elapsed,
                    true,
                    &mut dl_marquee,
                    term_width,
                );

                dl.rendered_width = visible_width(&dl_rendered);
                dl.term_width = term_width;
                self.download_summary = Some(dl);
                self.last_rendered_width = visible_width(&install_rendered);
                self.last_term_width = term_width;

                print!(
                    "\r\x1b[K{}\n\r\x1b[K  {}│{}\n\r\x1b[K{}",
                    dl_rendered, DIM_GREY, RESET, install_rendered
                );
                let _ = io::stdout().flush();
                return;
            }
        }

        if self.last_rendered_width > 0 && term_width < self.last_term_width {
            let extra_rows = self.last_rendered_width.saturating_sub(1) / term_width;
            if extra_rows > 0 {
                clear_wrapped_rows(extra_rows);
            }
        }

        self.last_rendered_width = visible_width(&install_rendered);
        self.last_term_width = term_width;

        print!("\r\x1b[K{}", install_rendered);
        let _ = io::stdout().flush();
    }
}

/// Renders a single formatted download progress line (orange) fitting strictly within `term_width - 1` columns.
pub fn render_progress_line(
    downloaded: u64,
    total: Option<u64>,
    speed: f64,
    elapsed: Duration,
    is_done: bool,
    marquee_pos: &mut usize,
    term_width: usize,
) -> String {
    render_colored_progress_line(
        downloaded,
        total,
        speed,
        elapsed,
        is_done,
        marquee_pos,
        term_width,
        ORANGE,
        DOWNLOAD_INDENT,
    )
}

/// Renders a single formatted installation progress line (green) fitting strictly within `term_width - 1` columns.
pub fn render_install_progress_line(
    installed: u64,
    total: Option<u64>,
    speed: f64,
    elapsed: Duration,
    is_done: bool,
    marquee_pos: &mut usize,
    term_width: usize,
) -> String {
    render_colored_progress_line(
        installed,
        total,
        speed,
        elapsed,
        is_done,
        marquee_pos,
        term_width,
        GREEN,
        INSTALL_INDENT,
    )
}

/// Renders a single formatted progress line with the specified bar color and indent fitting strictly within `term_width - 1` columns.
#[allow(clippy::too_many_arguments)]
pub fn render_colored_progress_line(
    downloaded: u64,
    total: Option<u64>,
    speed: f64,
    elapsed: Duration,
    is_done: bool,
    marquee_pos: &mut usize,
    term_width: usize,
    bar_color: &str,
    indent: &str,
) -> String {
    let indent_width = visible_width(indent);

    let prefix = if is_done {
        "\x1b[32m✓\x1b[0m 100%".to_string()
    } else if let Some(tot) = total {
        let pct = if tot > 0 {
            ((downloaded as f64 / tot as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };
        format!("{:>5.0}%", pct)
    } else {
        "   --%".to_string()
    };
    let prefix_width = visible_width(&prefix);

    // Build fixed-width right metrics string so bar_len never shifts between download & install
    // or between indeterminate, in-progress, and completed states:
    // - mb_str:    14 chars ("{:>5.1}/{:>5.1} MB" or "{:>5.1}/ --.- MB")
    // - speed_str: 10 chars ("{:>10}")
    // - time_str:   6 chars ("{:>6}" ETA or "{:>5.1}s" elapsed)
    // Total right_text width: 14 + 3 + 10 + 3 + 6 = 36 chars
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let downloaded_mb = downloaded as f64 / 1_048_576.0;
    let mb_str = match total {
        Some(tot) => {
            let tot_mb = (if is_done { tot.max(downloaded) } else { tot }) as f64 / 1_048_576.0;
            format!("{:>5.1}/{:>5.1} MB", downloaded_mb, tot_mb)
        }
        None if is_done => format!("{:>5.1}/{:>5.1} MB", downloaded_mb, downloaded_mb),
        None => format!("{:>5.1}/ --.- MB", downloaded_mb),
    };

    let effective_speed = if is_done {
        downloaded as f64 / elapsed_secs
    } else {
        speed
    };
    let speed_str = format!("{:>10}", format_speed(effective_speed));

    let time_str = if is_done {
        if elapsed_secs < 100.0 {
            format!("{:>5.1}s", elapsed_secs)
        } else {
            format!("{:>6}", format_eta(elapsed_secs))
        }
    } else if let Some(tot) = total {
        if speed > 0.0 && tot > downloaded {
            let remaining = (tot - downloaded) as f64 / speed;
            format!("{:>6}", format_eta(remaining))
        } else {
            " --:--".to_string()
        }
    } else {
        " --:--".to_string()
    };

    let right_text = format!("{} • {} • {}", mb_str, speed_str, time_str);

    // Calculate available space for the bar
    // Frame when right_text is present: indent_width + prefix_width + 1 (space) + 1 ('[') + bar_len + 1 (']') + 1 (space) + right_text_width
    let right_width = visible_width(&right_text);
    let full_overhead = indent_width + 4 + prefix_width + right_width;
    let max_avail = term_width.saturating_sub(1);

    // Adjust right text if terminal is narrow
    let (final_right, bar_len) = if max_avail >= full_overhead + 8 {
        let bar = max_avail - full_overhead;
        (right_text, bar)
    } else {
        // Fallback: compact fixed-width right text (13 chars: "{:>5.1}/{:>5.1}MB")
        let compact_mb = match total {
            Some(tot) => {
                let tot_mb = (if is_done { tot.max(downloaded) } else { tot }) as f64 / 1_048_576.0;
                format!("{:>5.1}/{:>5.1}MB", downloaded_mb, tot_mb)
            }
            None if is_done => format!("{:>5.1}/{:>5.1}MB", downloaded_mb, downloaded_mb),
            None => format!("{:>5.1}/ --.-MB", downloaded_mb),
        };
        let compact_width = visible_width(&compact_mb);
        let compact_overhead = indent_width + 4 + prefix_width + compact_width;
        if max_avail >= compact_overhead + 5 {
            let bar = max_avail - compact_overhead;
            (compact_mb, bar)
        } else {
            // Ultra-compact: no right text
            let minimal_overhead = indent_width + 2 + prefix_width;
            let bar = max_avail.saturating_sub(minimal_overhead).max(3);
            (String::new(), bar)
        }
    };

    // Render bar content
    let bar_content = if let Some(tot) = total {
        let ratio = if is_done {
            1.0
        } else if tot > 0 {
            (downloaded as f64 / tot as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let filled_len = ((ratio * bar_len as f64).round() as usize).min(bar_len);
        let unfilled_len = bar_len - filled_len;
        let filled = "█".repeat(filled_len);
        let unfilled = "░".repeat(unfilled_len);
        format!("{}{}{}{}{}", bar_color, filled, DIM_GREY, unfilled, RESET)
    } else if is_done {
        let filled = "█".repeat(bar_len);
        format!("{}{}{}", bar_color, filled, RESET)
    } else {
        // Marquee animation for indeterminate streams
        let pulse_len = 4.min(bar_len);
        let max_pos = bar_len.saturating_sub(pulse_len);
        let pos = if max_pos > 0 {
            *marquee_pos % (max_pos + 1)
        } else {
            0
        };
        *marquee_pos += 1;
        let left_unfilled = "░".repeat(pos);
        let pulse = "█".repeat(pulse_len);
        let right_unfilled = "░".repeat(bar_len.saturating_sub(pos + pulse_len));
        format!(
            "{}{}{}{}{}{}{}",
            DIM_GREY, left_unfilled, bar_color, pulse, DIM_GREY, right_unfilled, RESET
        )
    };

    if final_right.is_empty() {
        format!("{}{} {}[{}]{}", indent, prefix, DIM_GREY, bar_content, RESET)
    } else {
        format!(
            "{}{} {}[{}{}] {}{}",
            indent, prefix, DIM_GREY, bar_content, DIM_GREY, final_right, RESET
        )
    }
}

/// Computes visible character width of a string excluding ANSI escape sequences.
pub fn visible_width(s: &str) -> usize {
    let mut in_escape = false;
    let mut count = 0;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            count += 1;
        }
    }
    count
}

pub fn get_terminal_width() -> usize {
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        (w as usize).max(20)
    } else {
        80
    }
}

pub fn format_speed(bps: f64) -> String {
    if bps >= 1_073_741_824.0 {
        format!("{:.1} GB/s", bps / 1_073_741_824.0)
    } else if bps >= 1_048_576.0 {
        format!("{:.1} MB/s", bps / 1_048_576.0)
    } else if bps >= 1024.0 {
        format!("{:.1} KB/s", bps / 1024.0)
    } else {
        format!("{:.0} B/s", bps.max(0.0))
    }
}

pub fn format_eta(seconds: f64) -> String {
    if seconds < 0.0 || !seconds.is_finite() {
        return "--:--".to_string();
    }
    let total_secs = seconds.round() as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours >= 100 {
        ">99h ".to_string()
    } else if hours > 0 {
        format!("{:>2}h{:02}m", hours, mins)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_speed() {
        assert_eq!(format_speed(500.0), "500 B/s");
        assert_eq!(format_speed(1024.0), "1.0 KB/s");
        assert_eq!(format_speed(5_242_880.0), "5.0 MB/s");
    }

    #[test]
    fn test_format_eta() {
        assert_eq!(format_eta(4.0), "00:04");
        assert_eq!(format_eta(65.0), "01:05");
        assert_eq!(format_eta(3665.0), " 1h01m");
        assert_eq!(format_eta(100_000.0), "27h46m");
        assert_eq!(format_eta(-1.0), "--:--");
        assert_eq!(format_eta(f64::NAN), "--:--");
    }

    #[test]
    fn test_render_progress_line_bounds() {
        let mut marquee = 0;
        let line = render_progress_line(
            15_000_000,
            Some(30_000_000),
            5_000_000.0,
            Duration::from_secs(3),
            false,
            &mut marquee,
            80,
        );

        // Strip ANSI escapes to verify visible character length
        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let visible = re.replace_all(&line, "");
        assert!(visible.chars().count() < 80);
        assert!(visible.contains("50%"));
        assert!(visible.contains("MB"));
    }

    #[test]
    fn test_render_progress_line_completion() {
        let mut marquee = 0;
        let line = render_progress_line(
            30_000_000,
            Some(30_000_000),
            5_000_000.0,
            Duration::from_secs(6),
            true,
            &mut marquee,
            80,
        );

        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let visible = re.replace_all(&line, "");
        assert!(visible.contains("✓ 100%"));
        assert!(visible.chars().count() < 80);
    }

    #[test]
    fn test_render_progress_line_scaling() {
        let mut marquee = 0;
        for width in [40, 50, 60, 80, 100, 140] {
            let line = render_progress_line(
                10_000_000,
                Some(25_000_000),
                2_500_000.0,
                Duration::from_secs(4),
                false,
                &mut marquee,
                width,
            );
            let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
            let visible = re.replace_all(&line, "");
            assert!(
                visible.chars().count() < width,
                "Failed for width {}: line length {} was not < {}",
                width,
                visible.chars().count(),
                width
            );
        }
    }

    #[test]
    fn test_render_progress_indeterminate() {
        let mut marquee = 0;
        let line = render_progress_line(
            5_000_000,
            None,
            1_000_000.0,
            Duration::from_secs(5),
            false,
            &mut marquee,
            80,
        );
        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let visible = re.replace_all(&line, "");
        assert!(visible.contains("--%"));
        assert!(visible.chars().count() < 80);
    }

    #[test]
    fn test_render_visual_output() {
        let mut marquee = 0;
        let total = 37_580_960u64; // ~35.8 MB
        for (downloaded, is_done) in [
            (0, false),
            (9_395_240, false),
            (18_790_480, false),
            (28_185_720, false),
            (37_580_960, true),
        ] {
            let line = render_progress_line(
                downloaded,
                Some(total),
                5_400_000.0,
                Duration::from_secs(3),
                is_done,
                &mut marquee,
                80,
            );
            println!("{}", line);
        }
    }

    #[test]
    fn test_render_progress_line_stable_bar_length_across_speeds() {
        let mut marquee = 0;
        let speeds = [
            0.0,
            500.0,
            1024.0,
            1_048_576.0,     // 1 MB/s
            10_485_760.0,    // 10 MB/s
            104_857_600.0,   // 100 MB/s
            1_073_741_824.0, // 1 GB/s
        ];
        let mut expected_bar_len = None;
        for &spd in &speeds {
            let line = render_progress_line(
                50_000_000,
                Some(100_000_000),
                spd,
                Duration::from_secs(5),
                false,
                &mut marquee,
                100,
            );
            let start = line.find('[').unwrap();
            let end = line.find(']').unwrap();
            let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
            let bar_inner = re.replace_all(&line[start + 1..end], "");
            let bar_len = bar_inner.chars().count();
            if let Some(expected) = expected_bar_len {
                assert_eq!(
                    bar_len, expected,
                    "Bar length changed from {} to {} at speed {}",
                    expected, bar_len, spd
                );
            } else {
                expected_bar_len = Some(bar_len);
            }
        }
    }

    #[test]
    fn test_render_progress_line_stable_bar_length_across_progress() {
        let mut marquee = 0;
        let total = 120_000_000u64; // ~114.4 MB
        let bytes_points = [
            100_000,     // 0.1 MB
            1_000_000,   // 1.0 MB
            9_900_000,   // 9.4 MB
            10_500_000,  // 10.0 MB
            99_000_000,  // 94.4 MB
            110_000_000, // 104.9 MB
            119_000_000, // 113.5 MB
        ];
        let mut expected_bar_len = None;
        for &dl in &bytes_points {
            let line = render_progress_line(
                dl,
                Some(total),
                5_000_000.0,
                Duration::from_secs(5),
                false,
                &mut marquee,
                100,
            );
            let start = line.find('[').unwrap();
            let end = line.find(']').unwrap();
            let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
            let bar_inner = re.replace_all(&line[start + 1..end], "");
            let bar_len = bar_inner.chars().count();
            if let Some(expected) = expected_bar_len {
                assert_eq!(
                    bar_len, expected,
                    "Bar length changed from {} to {} at downloaded bytes {}",
                    expected, bar_len, dl
                );
            } else {
                expected_bar_len = Some(bar_len);
            }
        }
    }

    #[test]
    fn test_render_install_progress_line_green_color_and_bounds() {
        let mut marquee = 0;
        let line = render_install_progress_line(
            45_000_000,
            Some(90_000_000),
            30_000_000.0,
            Duration::from_millis(1500),
            false,
            &mut marquee,
            80,
        );

        assert!(line.contains(GREEN));
        assert!(!line.contains(ORANGE));

        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let visible = re.replace_all(&line, "");
        assert!(visible.chars().count() < 80);
        assert!(visible.contains("50%"));
        assert!(visible.contains("MB"));
    }

    #[test]
    fn test_render_install_progress_line_completion() {
        let mut marquee = 0;
        let line = render_install_progress_line(
            90_000_000,
            Some(90_000_000),
            45_000_000.0,
            Duration::from_secs(2),
            true,
            &mut marquee,
            80,
        );

        assert!(line.contains(GREEN));
        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let visible = re.replace_all(&line, "");
        assert!(visible.contains("✓ 100%"));
        assert!(visible.chars().count() < 80);
    }

    #[test]
    fn test_download_and_install_bars_always_equal_length() {
        let mut marquee = 0;
        let re = regex::Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").unwrap();
        let cases = [
            // (current_bytes, total_bytes, speed, elapsed, is_done)
            (0u64, None, 0.0, Duration::from_millis(100), false),
            (15_000_000, Some(38_400_000), 24_000_000.0, Duration::from_millis(600), false),
            (38_400_000, Some(38_400_000), 24_000_000.0, Duration::from_millis(12400), true),
            (50_000_000, Some(112_500_000), 142_000_000.0, Duration::from_millis(400), false),
            (112_500_000, Some(112_500_000), 142_000_000.0, Duration::from_millis(700), true),
            (450_000_000, Some(450_000_000), 200_000_000.0, Duration::from_millis(2200), true),
        ];

        for term_width in [60, 80, 100, 120] {
            let mut expected_bar_len = None;
            for &(cur, tot, spd, elapsed, is_done) in &cases {
                let dl_line = render_progress_line(cur, tot, spd, elapsed, is_done, &mut marquee, term_width);
                let inst_line = render_install_progress_line(cur, tot, spd, elapsed, is_done, &mut marquee, term_width);

                for line in [&dl_line, &inst_line] {
                    let start = line.find('[').unwrap();
                    let end = line.find(']').unwrap();
                    let bar_inner = re.replace_all(&line[start + 1..end], "");
                    let bar_len = bar_inner.chars().count();
                    if let Some(exp) = expected_bar_len {
                        assert_eq!(
                            bar_len, exp,
                            "Bar length changed at term_width={} for case {:?}",
                            term_width, (cur, tot, is_done)
                        );
                    } else {
                        expected_bar_len = Some(bar_len);
                    }
                }
            }
        }
    }
}


