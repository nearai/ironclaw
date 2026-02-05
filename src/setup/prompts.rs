//! Interactive prompt utilities for the setup wizard.
//!
//! Provides terminal UI components for:
//! - Single selection menus
//! - Multi-select with toggles
//! - Password/secret input (hidden)
//! - Yes/no confirmations
//! - Styled headers and step indicators

use std::io::{self, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType},
};
use secrecy::SecretString;

/// Display a numbered menu and get user selection.
///
/// Returns the index (0-based) of the selected option.
///
/// # Example
///
/// ```ignore
/// let choice = select_one("Choose an option:", &["Option A", "Option B"]);
/// ```
pub fn select_one(prompt: &str, options: &[&str]) -> io::Result<usize> {
    let mut stdout = io::stdout();

    // Print prompt
    writeln!(stdout, "{}", prompt)?;
    writeln!(stdout)?;

    // Print options
    for (i, option) in options.iter().enumerate() {
        writeln!(stdout, "  [{}] {}", i + 1, option)?;
    }
    writeln!(stdout)?;

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // Handle empty input as first option
        if input.is_empty() {
            return Ok(0);
        }

        // Parse number
        if let Ok(num) = input.parse::<usize>() {
            if num >= 1 && num <= options.len() {
                return Ok(num - 1);
            }
        }

        writeln!(
            stdout,
            "Invalid choice. Please enter a number 1-{}.",
            options.len()
        )?;
    }
}

/// Multi-select with space to toggle, enter to confirm.
///
/// `options` is a slice of (label, initially_selected) tuples.
/// Returns indices of selected options.
///
/// # Example
///
/// ```ignore
/// let selected = select_many("Select channels:", &[
///     ("CLI/TUI", true),
///     ("HTTP webhook", false),
///     ("Telegram", false),
/// ])?;
/// ```
pub fn select_many(prompt: &str, options: &[(&str, bool)]) -> io::Result<Vec<usize>> {
    let mut stdout = io::stdout();
    let mut selected: Vec<bool> = options.iter().map(|(_, s)| *s).collect();
    let mut cursor_pos = 0;

    terminal::enable_raw_mode()?;
    execute!(stdout, cursor::Hide)?;

    let result = (|| {
        loop {
            // Clear and redraw
            execute!(stdout, cursor::MoveToColumn(0))?;

            writeln!(stdout, "{}\r", prompt)?;
            writeln!(stdout, "\r")?;
            writeln!(
                stdout,
                "  (Use arrow keys to navigate, space to toggle, enter to confirm)\r"
            )?;
            writeln!(stdout, "\r")?;

            for (i, (label, _)) in options.iter().enumerate() {
                let checkbox = if selected[i] { "[x]" } else { "[ ]" };
                let prefix = if i == cursor_pos { ">" } else { " " };

                if i == cursor_pos {
                    execute!(stdout, SetForegroundColor(Color::Cyan))?;
                    writeln!(stdout, "  {} {} {}\r", prefix, checkbox, label)?;
                    execute!(stdout, ResetColor)?;
                } else {
                    writeln!(stdout, "  {} {} {}\r", prefix, checkbox, label)?;
                }
            }

            stdout.flush()?;

            // Read key
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Up => {
                        cursor_pos = cursor_pos.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        if cursor_pos < options.len() - 1 {
                            cursor_pos += 1;
                        }
                    }
                    KeyCode::Char(' ') => {
                        selected[cursor_pos] = !selected[cursor_pos];
                    }
                    KeyCode::Enter => {
                        break;
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    _ => {}
                }

                // Move cursor up to redraw
                execute!(
                    stdout,
                    cursor::MoveUp((options.len() + 4) as u16),
                    terminal::Clear(ClearType::FromCursorDown)
                )?;
            }
        }
        Ok(())
    })();

    // Cleanup
    execute!(stdout, cursor::Show)?;
    terminal::disable_raw_mode()?;
    writeln!(stdout)?;

    result?;

    Ok(selected
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| if s { Some(i) } else { None })
        .collect())
}

/// Password/secret input with hidden characters.
///
/// # Example
///
/// ```ignore
/// let token = secret_input("Bot token")?;
/// ```
pub fn secret_input(prompt: &str) -> io::Result<SecretString> {
    let mut stdout = io::stdout();

    print!("{}: ", prompt);
    stdout.flush()?;

    terminal::enable_raw_mode()?;
    let result = read_secret_line();
    terminal::disable_raw_mode()?;

    writeln!(stdout)?;
    result
}

fn read_secret_line() -> io::Result<SecretString> {
    let mut input = String::new();
    let mut stdout = io::stdout();

    loop {
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        {
            match code {
                KeyCode::Enter => {
                    break;
                }
                KeyCode::Backspace => {
                    if !input.is_empty() {
                        input.pop();
                        execute!(stdout, Print("\x08 \x08"))?;
                        stdout.flush()?;
                    }
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    execute!(stdout, Print('*'))?;
                    stdout.flush()?;
                }
                _ => {}
            }
        }
    }

    Ok(SecretString::from(input))
}

/// Yes/no confirmation prompt.
///
/// # Example
///
/// ```ignore
/// if confirm("Enable Telegram channel?", false)? {
///     // ...
/// }
/// ```
pub fn confirm(prompt: &str, default: bool) -> io::Result<bool> {
    let mut stdout = io::stdout();

    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{} {} ", prompt, hint);
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    Ok(match input.as_str() {
        "" => default,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

/// Print a styled header box.
///
/// # Example
///
/// ```ignore
/// print_header("IronClaw Setup Wizard");
/// ```
pub fn print_header(text: &str) {
    let width = text.len() + 4;
    let border = "─".repeat(width);

    println!();
    println!("╭{}╮", border);
    println!("│  {}  │", text);
    println!("╰{}╯", border);
    println!();
}

/// Print a step indicator.
///
/// # Example
///
/// ```ignore
/// print_step(1, 3, "NEAR AI Authentication");
/// // Output: Step 1/3: NEAR AI Authentication
/// //         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
/// ```
pub fn print_step(current: usize, total: usize, name: &str) {
    println!("Step {}/{}: {}", current, total, name);
    println!("{}", "━".repeat(32));
    println!();
}

/// Print a success message with checkmark.
pub fn print_success(message: &str) {
    println!("✓ {}", message);
}

/// Print an error message.
pub fn print_error(message: &str) {
    eprintln!("✗ {}", message);
}

/// Print an info message.
pub fn print_info(message: &str) {
    println!("  {}", message);
}

/// Read a simple line of input with a prompt.
pub fn input(prompt: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    print!("{}: ", prompt);
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// Read an optional line of input (empty returns None).
pub fn optional_input(prompt: &str, hint: Option<&str>) -> io::Result<Option<String>> {
    let mut stdout = io::stdout();

    if let Some(h) = hint {
        print!("{} ({}): ", prompt, h);
    } else {
        print!("{}: ", prompt);
    }
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input.to_string()))
    }
}

#[cfg(test)]
mod tests {
    // Interactive tests are difficult to unit test, but we can test the non-interactive parts.

    #[test]
    fn test_header_length_calculation() {
        // Just verify it doesn't panic with various inputs
        super::print_header("Test");
        super::print_header("A longer header text");
        super::print_header("");
    }

    #[test]
    fn test_step_indicator() {
        super::print_step(1, 3, "Test Step");
        super::print_step(3, 3, "Final Step");
    }
}
