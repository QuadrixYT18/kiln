//! Terminal capabilities: color, interactivity and progress output.

use std::fmt::Display;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

use owo_colors::OwoColorize;

static COLOR: AtomicBool = AtomicBool::new(false);
static ASSUME_YES: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

/// Decides once at startup whether colored output is used.
///
/// Honors `NO_COLOR`, `CLICOLOR_FORCE`, `TERM=dumb`, and falls back to plain
/// text when stdout is not a terminal or ANSI cannot be enabled (old Windows
/// consoles).
pub fn init(choice: ColorChoice, assume_yes: bool) {
    ASSUME_YES.store(assume_yes, Ordering::Relaxed);
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let dumb = std::env::var("TERM").is_ok_and(|t| t == "dumb");
    let forced = std::env::var_os("CLICOLOR_FORCE").is_some_and(|v| !v.is_empty() && v != "0");
    let want = match choice {
        ColorChoice::Always => true,
        ColorChoice::Never => false,
        ColorChoice::Auto => !no_color && !dumb && (forced || std::io::stdout().is_terminal()),
    };
    let enabled = want && enable_ansi();
    COLOR.store(enabled, Ordering::Relaxed);
}

#[cfg(windows)]
fn enable_ansi() -> bool {
    enable_ansi_support::enable_ansi_support().is_ok()
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

pub fn color_enabled() -> bool {
    COLOR.load(Ordering::Relaxed)
}

/// True when both stdin and stdout are terminals and `--yes` was not given.
pub fn interactive() -> bool {
    !ASSUME_YES.load(Ordering::Relaxed) && std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

macro_rules! painter {
    ($name:ident, $method:ident) => {
        pub fn $name(s: impl Display) -> String {
            if color_enabled() { s.$method().to_string() } else { s.to_string() }
        }
    };
}

painter!(green, green);
painter!(yellow, yellow);
painter!(red, red);
painter!(cyan, cyan);
painter!(bold, bold);
painter!(dim, dimmed);

/// Prefix glyphs fall back to ASCII when colors (and thus likely Unicode
/// support) are off.
pub fn ok_mark() -> String {
    if color_enabled() { green("✔") } else { "ok".to_string() }
}

pub fn warn_mark() -> String {
    if color_enabled() { yellow("!") } else { "warning:".to_string() }
}

pub fn err_mark() -> String {
    if color_enabled() { red("✘") } else { "error:".to_string() }
}

pub fn arrow() -> &'static str {
    if color_enabled() { "→" } else { "->" }
}

/// Width of a string ignoring ANSI escape sequences (for table alignment).
pub fn visible_width(s: &str) -> usize {
    let mut width = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

pub fn pad(s: &str, width: usize) -> String {
    let w = visible_width(s);
    if w >= width { s.to_string() } else { format!("{s}{}", " ".repeat(width - w)) }
}

pub fn spinner(msg: impl Into<String>) -> indicatif::ProgressBar {
    let pb = if std::io::stderr().is_terminal() && !assume_yes_env() {
        indicatif::ProgressBar::new_spinner()
    } else {
        indicatif::ProgressBar::hidden()
    };
    pb.set_message(msg.into());
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

pub fn progress(len: u64, msg: impl Into<String>) -> indicatif::ProgressBar {
    let pb = if std::io::stderr().is_terminal() {
        indicatif::ProgressBar::new(len)
    } else {
        indicatif::ProgressBar::hidden()
    };
    pb.set_style(
        indicatif::ProgressStyle::with_template("{msg} [{bar:30}] {pos}/{len}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
            .progress_chars("=> "),
    );
    pb.set_message(msg.into());
    pb
}

fn assume_yes_env() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_ignores_ansi() {
        assert_eq!(visible_width("\u{1b}[32mabc\u{1b}[0m"), 3);
        assert_eq!(visible_width("abc"), 3);
    }

    #[test]
    fn pad_uses_visible_width() {
        assert_eq!(pad("\u{1b}[31mab\u{1b}[0m", 4), "\u{1b}[31mab\u{1b}[0m  ");
    }
}
