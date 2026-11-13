//! Status from the screen: the generic heuristics, and reading/answering permission
//! prompts. Hook events (see `hooks`) always override these.
//!
//! Agents with full-screen TUIs (Claude Code, Codex, Antigravity) show permission prompts as
//! a numbered list with a selection marker, e.g.
//!
//! ```text
//!  Do you want to proceed?
//!  ❯ 1. Yes
//!    2. Yes, and don't ask again for npm test commands in this project
//!    3. No, and tell Claude what to do differently (esc)
//! ```
//!
//! Their list widgets move the selection with the arrow keys, confirm with Enter, and cancel
//! with Esc (see docs/agent-notes.md for what was checked against live prompts).
//! So Termlab answers by moving from the marked option to the wanted one and pressing Enter,
//! and denies with Esc. It never guesses positions: if the wanted option isn't on screen,
//! the answer is refused.

use std::sync::LazyLock;

use regex::Regex;

use crate::adapter::StatusHints;
use crate::model::{Approval, ApprovalChoice, ApprovalOption, Millis, SessionStatus};
use crate::pty::ScreenText;

/// Output within this window means the agent is working.
pub const RUNNING_WINDOW_MS: Millis = 2_000;

/// Prompts must sit within this many rows of the last non-empty row.
const PROMPT_ZONE_ROWS: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPrompt {
    pub approval: Approval,
    /// Number of the option currently marked as selected.
    pub selected: u8,
}

/// Classifies an option label into the button it corresponds to.
pub fn classify(label: &str) -> Option<ApprovalChoice> {
    let l = label.to_lowercase();
    let l = l.trim_start_matches(|c: char| !c.is_alphanumeric());
    let positive = [
        "yes", "allow", "approve", "proceed", "accept", "run", "trust",
    ];
    let negative = [
        "no", "deny", "reject", "cancel", "decline", "don't", "do not", "exit",
    ];
    if negative.iter().any(|w| starts_with_word(l, w)) {
        return Some(ApprovalChoice::Deny);
    }
    // "Yes, and switch to auto mode" changes the session's mode: no button maps to it.
    if l.contains("switch to") || l.contains(" mode") {
        return None;
    }
    if positive.iter().any(|w| starts_with_word(l, w)) {
        let always = [
            "don't ask again",
            "do not ask again",
            "always",
            "for this session",
            "during this session",
            "for the rest",
            "and remember",
        ];
        return Some(if always.iter().any(|w| l.contains(w)) {
            ApprovalChoice::AllowAlways
        } else {
            ApprovalChoice::AllowOnce
        });
    }
    None
}

fn starts_with_word(s: &str, w: &str) -> bool {
    s.strip_prefix(w)
        .is_some_and(|rest| rest.is_empty() || !rest.starts_with(|c: char| c.is_alphanumeric()))
}

/// Selection markers TUIs draw next to the highlighted option.
const MARKERS: &[char] = &['❯', '›', '▶', '▸', '➤', '●', '>'];
/// Box-drawing characters that may frame a prompt.
const FRAME: &str = "│┃|╎║";

/// Finds a permission prompt near the bottom of the screen: a list with a selection marker
/// (numbered or not), or failing that a numbered list.
pub fn parse_prompt(rows: &[String]) -> Option<ParsedPrompt> {
    let last = rows.iter().rposition(|r| !r.trim().is_empty())?;
    let zone_start = last.saturating_sub(PROMPT_ZONE_ROWS);
    let (first_row, options, selected) =
        parse_marked(rows, zone_start, last).or_else(|| parse_numbered(rows, zone_start, last))?;
    if options.len() < 2
        || !options.iter().any(|o| {
            matches!(
                o.choice,
                Some(ApprovalChoice::AllowOnce | ApprovalChoice::AllowAlways)
            )
        })
    {
        return None;
    }
    Some(ParsedPrompt {
        approval: Approval {
            question: question_above(rows, first_row),
            detail: None,
            options,
        },
        selected,
    })
}

/// The column where a row's text starts, if everything before it is space or frame.
fn text_start(row: &[char]) -> Option<usize> {
    row.iter()
        .position(|c| !(c.is_whitespace() || FRAME.contains(*c)))
}

/// `(marker column, text column)` for a row whose first visible char is a selection marker.
fn marker_cols(row: &[char]) -> Option<(usize, usize)> {
    let m = text_start(row)?;
    if !MARKERS.contains(&row[m]) {
        return None;
    }
    let t = (m + 1..row.len()).find(|&i| !row[i].is_whitespace())?;
    Some((m, t))
}

/// Strips a leading "1. " / "2) " from an option label.
fn strip_number(label: &str) -> &str {
    let t = label.trim_start();
    let digits = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && let Some(rest) = t[digits..].strip_prefix(['.', ')'])
        && rest.starts_with(' ')
    {
        return rest.trim_start();
    }
    t
}

fn option_label(row: &[char], col: usize) -> String {
    let s: String = row[col..].iter().collect();
    strip_number(s.trim_end_matches(|c: char| c.is_whitespace() || FRAME.contains(c))).to_string()
}

/// A list anchored on the selection marker: sibling options start in the same column.
/// Rows indented further are wrapped text of the option above; anything else ends the list.
fn parse_marked(
    rows: &[String],
    zone_start: usize,
    last: usize,
) -> Option<(usize, Vec<ApprovalOption>, u8)> {
    let grid: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
    let (marker_row, text_col) = (zone_start..=last)
        .rev()
        .find_map(|i| marker_cols(&grid[i]).map(|(_, t)| (i, t)))?;
    let is_option = |i: usize| {
        text_start(&grid[i]) == Some(text_col)
            || marker_cols(&grid[i]).is_some_and(|(_, t)| t == text_col)
    };
    let is_continuation = |i: usize| text_start(&grid[i]).is_some_and(|c| c > text_col);

    let mut first = marker_row;
    let mut i = marker_row;
    while i > 0 {
        let up = i - 1;
        if is_option(up) {
            first = up;
            i = up;
        } else if is_continuation(up) && up > 0 && (is_option(up - 1) || is_continuation(up - 1)) {
            i = up;
        } else {
            break;
        }
    }
    let mut end = marker_row;
    let mut j = marker_row + 1;
    while j <= last {
        if is_option(j) {
            end = j;
        } else if !is_continuation(j) {
            break;
        }
        j += 1;
    }
    let mut options = Vec::new();
    let mut selected = 1;
    for (r, row) in grid.iter().enumerate().take(end + 1).skip(first) {
        if !is_option(r) {
            continue;
        }
        let label = option_label(row, text_col);
        let number = (options.len() + 1) as u8;
        if r == marker_row {
            selected = number;
        }
        options.push(ApprovalOption {
            number,
            choice: classify(&label),
            label,
        });
    }
    Some((first, options, selected))
}

static OPTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[\s│┃|╎║]*(?P<n>[1-9])[.)]\s+(?P<label>.+?)[\s│┃|╎║]*$").expect("valid regex")
});

/// Fallback without a visible marker: the lowest run of options numbered 1..k.
fn parse_numbered(
    rows: &[String],
    zone_start: usize,
    last: usize,
) -> Option<(usize, Vec<ApprovalOption>, u8)> {
    let mut found: Vec<(usize, ApprovalOption)> = Vec::new();
    for idx in (zone_start..=last).rev() {
        let row = &rows[idx];
        if let Some(c) = OPTION_RE.captures(row) {
            let n: u8 = c["n"].parse().ok()?;
            if found.last().is_some_and(|(_, o)| o.number != n + 1) {
                if found.last().is_some_and(|(_, o)| o.number == 1) {
                    break;
                }
                found.clear();
            }
            let label = c["label"].trim().to_string();
            found.push((
                idx,
                ApprovalOption {
                    number: n,
                    choice: classify(&label),
                    label,
                },
            ));
            if n == 1 {
                break;
            }
        } else if found.last().is_some_and(|(_, o)| o.number == 1) {
            break;
        }
    }
    found.reverse();
    if found.first().is_none_or(|(_, o)| o.number != 1) {
        return None;
    }
    let first = found[0].0;
    Some((first, found.into_iter().map(|(_, o)| o).collect(), 1))
}

/// The agent's question: the nearest row (within 12) above the list ending in "?", else one
/// containing "?" (cut after it), else the nearest non-empty row.
fn question_above(rows: &[String], first_row: usize) -> String {
    let above: Vec<&str> = rows[first_row.saturating_sub(12)..first_row]
        .iter()
        .map(|r| strip_box(r))
        .filter(|r| !r.is_empty())
        .collect();
    // Codex shows "Reason: …?" under its question; that's detail, not the question.
    let above: Vec<&str> = above
        .into_iter()
        .filter(|r| !r.starts_with("Reason:"))
        .collect();
    if let Some(q) = above.iter().rev().find(|r| r.ends_with('?')) {
        return q.to_string();
    }
    if let Some(q) = above.iter().rev().find(|r| r.contains('?')) {
        let cut = q.find('?').map(|i| i + 1).unwrap_or(q.len());
        let start = q[..cut].rfind([':', '.', '!']).map(|i| i + 1).unwrap_or(0);
        return q[start..cut].trim().to_string();
    }
    above.last().map(|s| s.to_string()).unwrap_or_default()
}

fn strip_box(row: &str) -> &str {
    row.trim_matches(|c: char| c.is_whitespace() || "│┃|╎║╭╮╰╯─━".contains(c))
}

/// The keystrokes that pick `choice` on the prompt currently on screen.
pub fn answer_keys(
    screen: &ScreenText,
    choice: ApprovalChoice,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    if choice == ApprovalChoice::Deny {
        return Some(b"\x1b".to_vec());
    }
    let prompt = parse_prompt(&screen.rows)?;
    let target = prompt
        .approval
        .options
        .iter()
        .find(|o| o.choice == Some(choice))?
        .number;
    let (up, down): (&[u8], &[u8]) = if application_cursor {
        (b"\x1bOA", b"\x1bOB")
    } else {
        (b"\x1b[A", b"\x1b[B")
    };
    let mut keys = Vec::new();
    let (key, n) = if target >= prompt.selected {
        (down, target - prompt.selected)
    } else {
        (up, prompt.selected - target)
    };
    for _ in 0..n {
        keys.extend_from_slice(key);
    }
    keys.push(b'\r');
    Some(keys)
}

/// A fingerprint of the visible text for activity detection. Braille-pattern characters
/// (U+2800–U+28FF) and whitespace are left out: TUIs animate braille art while idle (Vibe's
/// logo cells turn into blanks and back), and that alone doesn't mean the agent is working.
pub fn activity_hash(rows: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for row in rows {
        for c in row
            .chars()
            .filter(|c| !c.is_whitespace() && !('\u{2800}'..='\u{28FF}').contains(c))
        {
            c.hash(&mut h);
        }
        '\n'.hash(&mut h);
    }
    h.finish()
}

/// The fallback status for a live session without hook information.
pub fn heuristic_status(
    now: Millis,
    last_output_at: Millis,
    screen: &ScreenText,
    hints: &StatusHints,
) -> SessionStatus {
    // A prompt at the bottom of the screen wins over recent output: Codex keeps redrawing
    // while it waits for an answer. Once the agent moves on, new output pushes the prompt
    // out of the bottom zone (or a full-screen TUI clears it).
    if parse_prompt(&screen.rows).is_some() {
        return SessionStatus::NeedsYou;
    }
    if last_output_at > 0 && now - last_output_at < RUNNING_WINDOW_MS {
        return SessionStatus::Running;
    }
    if matches_hints(screen, hints) {
        return SessionStatus::NeedsYou;
    }
    SessionStatus::Idle
}

fn matches_hints(screen: &ScreenText, hints: &StatusHints) -> bool {
    let tail = screen.tail(6).join("\n").to_lowercase();
    hints
        .needs_input
        .iter()
        .any(|p| tail.contains(&p.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::AgentAdapter;

    fn screen(rows: &[&str]) -> ScreenText {
        ScreenText {
            rows: rows.iter().map(|s| s.to_string()).collect(),
            bracketed_paste: false,
            application_cursor: false,
        }
    }

    const CLAUDE: &[&str] = &[
        "● Bash(npm test)",
        "",
        "╭──────────────────────────────────────────────╮",
        "│ Bash command                                 │",
        "│                                              │",
        "│   npm test                                   │",
        "│                                              │",
        "│ Do you want to proceed?                      │",
        "│ ❯ 1. Yes                                     │",
        "│   2. Yes, and don't ask again for npm test   │",
        "│   3. No, and tell Claude what to do          │",
        "│      differently (esc)                       │",
        "╰──────────────────────────────────────────────╯",
        "",
    ];

    /// Captured from Claude Code 2.1.322 on a new folder (live probe, 2026-11-12).
    /// Unnumbered, and the highlighted default is "No, exit".
    const CLAUDE_TRUST: &[&str] = &[
        "──────────────────────────────────────────────────────",
        " Accessing workspace:",
        "",
        " /private/var/folders/nx/T/termlab-probe/project",
        "",
        " Quick safety check: Is this a project you created or one you trust? (Like your own code, a well-known open",
        " source project, or work from your team). If not, take a moment to review what's in this folder first.",
        "",
        " Claude Code'll be able to read, edit, and execute files here.",
        "",
        " Security guide",
        "",
        " ❯ No, exit",
        "   Yes, I trust this folder",
        "",
        " Enter to confirm · Esc to cancel",
        "",
        "",
    ];

    #[test]
    fn real_claude_trust_dialog_moves_off_the_no_default() {
        let s = screen(CLAUDE_TRUST);
        let p = parse_prompt(&s.rows).expect("prompt");
        assert_eq!(p.selected, 1);
        assert_eq!(
            p.approval.question,
            "Is this a project you created or one you trust?"
        );
        let labels: Vec<_> = p
            .approval
            .options
            .iter()
            .map(|o| (o.label.as_str(), o.choice))
            .collect();
        assert_eq!(
            labels,
            [
                ("No, exit", Some(ApprovalChoice::Deny)),
                ("Yes, I trust this folder", Some(ApprovalChoice::AllowOnce))
            ]
        );
        // "Allow once" must move down to "Yes" — pressing Enter alone would exit Claude.
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowOnce, false).unwrap(),
            b"\x1b[B\r"
        );
    }

    /// Captured from codex-cli 0.167.0 asking to leave the read-only sandbox (live probe,
    /// 2026-11-12). Codex keeps redrawing while it waits.
    const CODEX_APPROVAL: &[&str] = &[
        "• Running touch probe.txt",
        "",
        "",
        "  Would you like to run the following command?",
        "",
        "  Environment: local",
        "",
        "  Reason: May I run touch probe.txt outside the read-only sandbox to create the requested file?",
        "",
        "  $ touch probe.txt",
        "",
        "",
        "› 1. Yes, proceed (y)",
        "  2. Yes, and don't ask again for commands that start with `touch probe.txt` (p)",
        "  3. No, and tell Codex what to do differently (esc)",
        "",
        "  Press enter to confirm or esc to cancel",
        "",
    ];

    #[test]
    fn real_codex_approval_is_needs_you_even_while_redrawing() {
        let s = screen(CODEX_APPROVAL);
        let p = parse_prompt(&s.rows).expect("prompt");
        assert_eq!(
            p.approval.question,
            "Would you like to run the following command?"
        );
        let choices: Vec<_> = p.approval.options.iter().map(|o| o.choice).collect();
        assert_eq!(
            choices,
            [
                Some(ApprovalChoice::AllowOnce),
                Some(ApprovalChoice::AllowAlways),
                Some(ApprovalChoice::Deny)
            ]
        );
        assert_eq!(p.approval.options[0].label, "Yes, proceed (y)");
        let hints = StatusHints::default();
        // Output 100 ms ago, but the prompt is on screen.
        assert_eq!(
            heuristic_status(10_000, 9_900, &s, &hints),
            SessionStatus::NeedsYou
        );
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowAlways, false).unwrap(),
            b"\x1b[B\r"
        );
    }

    /// Vibe 3.0.3 idle in Full auto (auto-approve), captured 2026-11-12. The logo is
    /// animated, and the mode label says "auto approve".
    const VIBE_IDLE: &[&str] = &[
        "",
        "⢀⠤⢄ ⢀⠔⠒⠒⡤⡀⡠⡀  Mistral Vibe v3.0.3",
        " ⠉⠣⣓⡇  ⢠⠃⠨⡠⢣  Mistral Large 4[high] · [Subscription] Pro",
        "   ⠸⠤⠤⠤⠭⠯⠵⠭⠇  Type /help for more information",
        "───────────────────────────────────────────────────────────── auto approve ─",
        ">",
        "─────────────────────────────────────────────────────────────────────────────",
        "/Users/demo/code/project                  Mistral Large 4[high] · 0/400k tokens (0%)",
    ];

    #[test]
    fn idle_vibe_is_idle_and_its_animated_logo_is_not_activity() {
        let s = screen(VIBE_IDLE);
        let hints = crate::adapter::VibeAdapter.status_hints();
        assert_eq!(parse_prompt(&s.rows), None);
        assert_eq!(
            heuristic_status(10_000, 1_000, &s, &hints),
            SessionStatus::Idle
        );
        let mut frame2 = VIBE_IDLE.iter().map(|r| r.to_string()).collect::<Vec<_>>();
        frame2[1] = "    ⢀⠔⠒⠒⡔⢄⠔⡄  Mistral Vibe v3.0.3".into();
        assert_eq!(activity_hash(&s.rows), activity_hash(&frame2));
        frame2[5] = "> hello".into();
        assert_ne!(activity_hash(&s.rows), activity_hash(&frame2));
    }

    #[test]
    fn classifies_labels() {
        assert_eq!(classify("Yes"), Some(ApprovalChoice::AllowOnce));
        assert_eq!(
            classify("Yes, proceed (y)"),
            Some(ApprovalChoice::AllowOnce)
        );
        assert_eq!(
            classify("Yes, and don't ask again for this command"),
            Some(ApprovalChoice::AllowAlways)
        );
        assert_eq!(classify("Allow always"), Some(ApprovalChoice::AllowAlways));
        assert_eq!(
            classify("No, and tell Claude what to do differently (esc)"),
            Some(ApprovalChoice::Deny)
        );
        assert_eq!(classify("Nothing to see"), None);
        // From Claude Code 2.1.322's real permission prompt.
        assert_eq!(
            classify("Yes, and switch to auto mode · auto mode handles these prompts for you"),
            None
        );
        assert_eq!(classify("Notes"), None);
    }

    #[test]
    fn parses_a_claude_prompt() {
        let p = parse_prompt(&screen(CLAUDE).rows).expect("prompt");
        assert_eq!(p.selected, 1);
        assert_eq!(p.approval.question, "Do you want to proceed?");
        let choices: Vec<_> = p.approval.options.iter().map(|o| o.choice).collect();
        assert_eq!(
            choices,
            vec![
                Some(ApprovalChoice::AllowOnce),
                Some(ApprovalChoice::AllowAlways),
                Some(ApprovalChoice::Deny)
            ]
        );
    }

    #[test]
    fn parses_a_codex_style_prompt_with_selection_lower_down() {
        let rows = [
            "Would you like to run the following command?",
            "",
            "  $ cargo test",
            "",
            "  1. Yes, proceed (y)",
            "› 2. Yes, and don't ask again for this command (a)",
            "  3. No, and tell Codex what to do differently (esc)",
            "",
            "Press enter to confirm or esc to cancel",
        ];
        let s = screen(&rows);
        let p = parse_prompt(&s.rows).unwrap();
        assert_eq!(p.selected, 2);
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowOnce, false).unwrap(),
            b"\x1b[A\r"
        );
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowAlways, true).unwrap(),
            b"\r"
        );
        assert_eq!(
            answer_keys(&s, ApprovalChoice::Deny, false).unwrap(),
            b"\x1b"
        );
    }

    #[test]
    fn answer_keys_move_down_from_the_marker() {
        let s = screen(CLAUDE);
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowOnce, false).unwrap(),
            b"\r"
        );
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowAlways, false).unwrap(),
            b"\x1b[B\r"
        );
        assert_eq!(
            answer_keys(&s, ApprovalChoice::AllowAlways, true).unwrap(),
            b"\x1bOB\r"
        );
    }

    #[test]
    fn missing_option_is_refused() {
        let rows = [
            "Do you trust the files in this folder?",
            "❯ 1. Yes, proceed",
            "  2. No, exit",
        ];
        let s = screen(&rows);
        assert!(parse_prompt(&s.rows).is_some());
        assert_eq!(answer_keys(&s, ApprovalChoice::AllowAlways, false), None);
    }

    #[test]
    fn ordinary_numbered_lists_are_not_prompts() {
        let rows = [
            "Here is the plan:",
            "1. Add a queue table",
            "2. Move retries to a worker",
            "3. Add tests",
            "",
            "> ",
        ];
        assert_eq!(parse_prompt(&screen(&rows).rows), None);
    }

    #[test]
    fn prompts_far_above_the_bottom_are_ignored() {
        let mut rows: Vec<&str> = CLAUDE.to_vec();
        rows.extend(std::iter::repeat_n("later output", 20));
        assert_eq!(parse_prompt(&screen(&rows).rows), None);
    }

    #[test]
    fn heuristics() {
        let hints = StatusHints {
            needs_input: vec!["(y/n)"],
        };
        let quiet = screen(&["$ ls", "done", "> "]);
        assert_eq!(
            heuristic_status(10_000, 9_000, &quiet, &hints),
            SessionStatus::Running
        );
        assert_eq!(
            heuristic_status(10_000, 1_000, &quiet, &hints),
            SessionStatus::Idle
        );
        assert_eq!(
            heuristic_status(10_000, 0, &quiet, &hints),
            SessionStatus::Idle
        );
        assert_eq!(
            heuristic_status(10_000, 1_000, &screen(CLAUDE), &hints),
            SessionStatus::NeedsYou
        );
        let yn = screen(&["Apply these edits? (y/n)"]);
        assert_eq!(
            heuristic_status(10_000, 1_000, &yn, &hints),
            SessionStatus::NeedsYou
        );
    }
}
