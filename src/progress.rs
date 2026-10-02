use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

const ORANGE: &str = "\x1b[38;2;255;140;0m";
const DIM_GREY: &str = "\x1b[38;2;90;90;90m";
const RESET: &str = "\x1b[0m";

/// Download progress bar with pip-like styling:
/// Classic block style: Solid orange blocks with a dim grey shaded background,
/// percentage on the left, downloaded/total MB, speed, and ETA on the right.
/// Dynamically scales to terminal width and stays on a single line.
pub struct DownloadProgress {
    total_bytes: Option<u64>,
    downloaded_bytes: u64,
    start_time: Instant,
    last_render_time: Instant,
    last_render_bytes: u64,
    smoothed_speed: f64,
    is_terminal: bool,
    last_marquee_pos: usize,
}

impl DownloadProgress {
    pub fn new(total_bytes: Option<u64>) -> Self {
        let is_terminal = io::stdout().is_terminal();
        let now = Instant::now();
        let mut progress = Self {
            total_bytes,
            downloaded_bytes: 0,
            start_time: now,
            last_render_time: now,
            last_render_bytes: 0,
            smoothed_speed: 0.0,
            is_terminal,
            last_marquee_pos: 0,
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

    /// Complete the progress display with 100% full orange bar and summary metrics.
    pub fn finish(&mut self) {
        if self.is_terminal {
            self.render(true);
            println!();
            let _ = io::stdout().flush();
        } else {
            let elapsed = self.start_time.elapsed().as_secs_f64().max(0.001);
            let mb = self.downloaded_bytes as f64 / 1_048_576.0;
            println!(
                "    ✓ Downloaded {:.1} MB in {:.1}s",
                mb, elapsed
            );
        }
    }

    fn render(&mut self, is_done: bool) {
        let term_width = get_terminal_width();
        let rendered = render_progress_line(
            self.downloaded_bytes,
            self.total_bytes,
            self.smoothed_speed,
            self.start_time.elapsed(),
            is_done,
            &mut self.last_marquee_pos,
            term_width,
        );

        // Carriage return \r + clear line \x1b[K guarantees no artifacts on terminal resize
        print!("\r\x1b[K{}", rendered);
        let _ = io::stdout().flush();
    }
}

/// Renders a single formatted line fitting strictly within `term_width - 1` columns.
pub fn render_progress_line(
    downloaded: u64,
    total: Option<u64>,
    speed: f64,
    elapsed: Duration,
    is_done: bool,
    marquee_pos: &mut usize,
    term_width: usize,
) -> String {
    let indent = "    "; // 4 spaces

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

    // Build right metrics string based on completeness and available space
    let elapsed_secs = elapsed.as_secs_f64().max(0.001);
    let right_text = if is_done {
        let total_mb = total.unwrap_or(downloaded) as f64 / 1_048_576.0;
        let downloaded_mb = downloaded as f64 / 1_048_576.0;
        let avg_speed = downloaded as f64 / elapsed_secs;
        if total.is_some() {
            let tot_str = format!("{:.1}", total_mb);
            let num_width = tot_str.len();
            format!(
                "{:>width$.1} / {} MB • {:>10} in {:.1}s",
                downloaded_mb,
                tot_str,
                format_speed(avg_speed),
                elapsed_secs,
                width = num_width
            )
        } else {
            format!(
                "{:>6.1} MB • {:>10} in {:.1}s",
                downloaded_mb,
                format_speed(avg_speed),
                elapsed_secs
            )
        }
    } else {
        let downloaded_mb = downloaded as f64 / 1_048_576.0;
        let mb_str = if let Some(tot) = total {
            let tot_mb = tot as f64 / 1_048_576.0;
            let tot_str = format!("{:.1}", tot_mb);
            let num_width = tot_str.len();
            format!("{:>width$.1} / {} MB", downloaded_mb, tot_str, width = num_width)
        } else {
            format!("{:>6.1} MB", downloaded_mb)
        };
        let speed_str = format!("{:>10}", format_speed(speed));
        let eta_str = if let Some(tot) = total {
            if speed > 0.0 && tot > downloaded {
                let remaining = (tot - downloaded) as f64 / speed;
                format!("{:>6}", format_eta(remaining))
            } else {
                " --:--".to_string()
            }
        } else {
            "".to_string()
        };

        if !eta_str.is_empty() {
            format!("{} • {} • {}", mb_str, speed_str, eta_str)
        } else {
            format!("{} • {}", mb_str, speed_str)
        }
    };

    // Calculate available space for the bar
    // Frame when right_text is present: 4 (indent) + prefix_width + 1 (space) + 1 ('[') + bar_len + 1 (']') + 1 (space) + right_text_width = 8 + prefix_width + right_text_width
    let right_width = visible_width(&right_text);
    let full_overhead = 8 + prefix_width + right_width;
    let max_avail = term_width.saturating_sub(1);

    // Adjust right text if terminal is narrow
    let (final_right, bar_len) = if max_avail >= full_overhead + 8 {
        let bar = max_avail - full_overhead;
        (right_text, bar)
    } else {
        // Fallback: compact right text (MB only)
        let compact_mb = if let Some(tot) = total {
            let tot_mb = tot as f64 / 1_048_576.0;
            let tot_str = format!("{:.1}", tot_mb);
            let num_width = tot_str.len();
            format!("{:>width$.1}/{}MB", downloaded as f64 / 1_048_576.0, tot_str, width = num_width)
        } else {
            format!("{:>5.1}MB", downloaded as f64 / 1_048_576.0)
        };
        let compact_width = visible_width(&compact_mb);
        let compact_overhead = 8 + prefix_width + compact_width;
        if max_avail >= compact_overhead + 5 {
            let bar = max_avail - compact_overhead;
            (compact_mb, bar)
        } else {
            // Ultra-compact: no right text
            let minimal_overhead = 6 + prefix_width; // 4 (indent) + prefix_width + 1 (space) + 1 ('[') + bar_len + 1 (']')
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
        format!("{}{}{}{}{}", ORANGE, filled, DIM_GREY, unfilled, RESET)
    } else if is_done {
        let filled = "█".repeat(bar_len);
        format!("{}{}{}", ORANGE, filled, RESET)
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
            DIM_GREY, left_unfilled, ORANGE, pulse, DIM_GREY, right_unfilled, RESET
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
}


