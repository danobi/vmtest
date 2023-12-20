use std::cmp::min;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use anyhow::{anyhow, bail, Error, Result};
use console::{strip_ansi_codes, style, truncate_str, Style, Term};
use regex::Regex;

use crate::output::Output;
use crate::vmtest::Vmtest;

const WINDOW_LENGTH: usize = 10;
// sysexits.h catchall exit code for when we failed to run the vm for miscellaneous reasons.
const EX_UNAVAILABLE: i32 = 69;

/// Console UI
///
/// This struct handles all the fancy pretty printing as well as
/// formatting and reporting any errors.
pub struct Ui {
    vmtest: Vmtest,
}

struct Stage {
    term: Term,
    lines: Vec<String>,
    expand: bool,
}

/// Helper to clear lines depending on whether or not a tty is attached
fn clear_last_lines(term: &Term, n: usize) {
    if term.features().is_attended() {
        term.clear_last_lines(n).unwrap();
    }
}

impl Stage {
    /// Start a new stage.
    ///
    /// We take ownership of the previous stage to control the drop order.
    /// Without this, something like:
    ///
    /// ```ignore
    /// stage = Some(Stage::new(..));
    /// ```
    ///
    /// would cause the new stage to print its heading first. Then the old
    /// stage's drop would clear the new stage's heading. Causing some
    /// visual corruption.
    ///
    /// By taking ownership of the old stage, we defuse this footgun through
    /// the API.
    fn new(term: Term, heading: &str, previous: Option<Stage>) -> Self {
        drop(previous);

        // I don't see how writing to terminal could fail, but if it does,
        // we have no choice but to panic anyways.
        term.write_line(heading).expect("Failed to write terminal");

        Self {
            term,
            lines: Vec::new(),
            expand: false,
        }
    }

    /// Returns the current active window size
    ///
    /// We kinda hack this to return 1 if we're not writing to terminal.
    /// Should really find a cleaner solution.
    fn window_size(&self) -> usize {
        if self.term.features().is_attended() {
            min(self.lines.len(), WINDOW_LENGTH)
        } else {
            min(self.lines.len(), 1)
        }
    }

    /// Add a line to the output window.
    ///
    /// If over the window size, older text will be cleared from the screen.
    ///
    /// Note we never expect printing to terminal to fail. Even if it did,
    /// we'd have no choice but to panic, so panic.
    fn print_line(&mut self, line: &str, custom: Option<Style>) {
        // Caller must take care that `line` is indeed a single line
        // Note we check <= 1 b/c an empty string is allowed but technically 0 lines.
        assert!(line.lines().count() <= 1, "Multiple lines provided");

        // Clear previously visible lines
        clear_last_lines(&self.term, self.window_size());

        // Compute new visible lines
        let trimmed_line = line.trim_end();
        let stripped_line = strip_ansi_codes(trimmed_line);
        let styled_line = match &custom {
            Some(s) => s.apply_to(stripped_line),
            None => style(stripped_line).dim(),
        };
        self.lines.push(styled_line.to_string());
        // Unwrap should never fail b/c we're sizing with `min()`
        let window = self.lines.windows(self.window_size()).last().unwrap();

        // Get terminal width, if any
        let width = match self.term.size_checked() {
            Some((_, w)) => w,
            _ => u16::MAX,
        };

        // Print visible lines
        for line in window {
            let clipped = truncate_str(line, width as usize - 3, "...");
            self.term.write_line(&format!("{}", clipped)).unwrap();
        }
    }

    /// If true, rather than clean up the output window, expand and show
    /// all the cached output
    ///
    /// Typically used when an error is met.
    fn expand(&mut self, b: bool) {
        self.expand = b;
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        clear_last_lines(&self.term, self.window_size());
        if self.expand && self.term.features().is_attended() {
            for line in &self.lines {
                self.term
                    .write_line(line)
                    .expect("Failed to write terminal");
            }
        }
    }
}

/// Returns an unstyled heading with provided depth
fn heading(name: &str, depth: usize) -> String {
    let middle = "=".repeat((depth - 1) * 2);
    format!("={}> {}", middle, name)
}

/// Wraps erroring out a stage
fn error_out_stage(stage: &mut Stage, err: &Error) {
    // NB: use debug formatting to get full trace
    let err = format!("{:?}", err);
    for line in err.lines() {
        stage.print_line(line, Some(Style::new().red().bright()));
    }
    stage.expand(true);
}

impl Ui {
    /// Construct a new UI
    pub fn new(vmtest: Vmtest) -> Self {
        Self { vmtest }
    }

    /// UI for a single target. Must be run on its own thread.
    ///
    /// Returns an Error if the vm failed to run the command.
    /// Otherwise, return the return code of the command.
    fn target_ui(updates: Receiver<Output>, target: String, show_cmd: bool) -> Result<i32> {
        let term = Term::stdout();
        let mut stage = Stage::new(term.clone(), &heading(&target, 1), None);
        let mut stages = 0;
        let mut errors = 0;
        let mut rc: i32 = 0;

        // Main state machine loop
        loop {
            let msg = match updates.recv() {
                Ok(l) => l,
                // Qemu hangs up when done
                Err(_) => break,
            };

            match &msg {
                Output::BootStart => {
                    stage = Stage::new(term.clone(), &heading("Booting", 2), Some(stage));
                    stages += 1;
                }
                Output::Boot(s) => stage.print_line(s, None),
                Output::BootEnd(r) => {
                    if let Err(e) = r {
                        error_out_stage(&mut stage, e);
                        errors += 1;
                    }
                }
                Output::SetupStart => {
                    stage = Stage::new(term.clone(), &heading("Setting up VM", 2), Some(stage));
                    stages += 1;
                }
                Output::Setup(s) => stage.print_line(s, None),
                Output::SetupEnd(r) => {
                    if let Err(e) = r {
                        error_out_stage(&mut stage, e);
                        errors += 1;
                    }
                }
                Output::CommandStart => {
                    stage = Stage::new(term.clone(), &heading("Running command", 2), Some(stage));
                    stages += 1;
                }
                Output::Command(s) => stage.print_line(s, None),
                Output::CommandEnd(r) => {
                    if show_cmd {
                        stage.expand(true);
                    }

                    match r {
                        Ok(retval) => {
                            rc = *retval as i32;
                            // Do not increment the error counter here. The VM ran successfully, the error is convyed by
                            // the command return code.
                            // Nevertheless, we still want to make it clear to the user by logging to the console.
                            if *retval != 0 {
                                error_out_stage(
                                    &mut stage,
                                    &anyhow!("Command failed with exit code: {}", retval),
                                );
                            }
                        }
                        Err(e) => {
                            error_out_stage(&mut stage, e);
                            errors += 1;
                        }
                    };
                }
            }
        }

        // Force stage cleanup so we can do final fixup if we want
        drop(stage);

        // Only clear target stages if target was successful
        if errors == 0 && !show_cmd {
            clear_last_lines(&term, stages);
            term.write_line("PASS").expect("Failed to write terminal");
        } else if !show_cmd {
            term.write_line("FAILED").expect("Failed to write terminal");
        }

        if errors != 0 {
            bail!("Failed to run the target");
        }
        Ok(rc)
    }

    /// Run all the targets in the provided `vmtest`
    ///
    /// `filter` specifies the regex to filter targets by.
    /// `show_cmd` specifies if the command output should always be shown.
    ///
    /// Note this function is "infallible" b/c on error it will display
    /// the appropriate error message to screen.
    /// In one-liner mode, it return the return code of the command, or EX_UNAVAILABLE if there
    /// is an issue that prevents running the command.
    /// When multiple targets are ran, it returns how many targets failed.
    pub fn run(self, filter: &Regex, show_cmd: bool) -> i32 {
        let mut failed = 0;
        let targets = self
            .vmtest
            .targets()
            .iter()
            .filter(|t| filter.is_match(&t.name))
            .collect::<Vec<_>>();
        let single_cmd = targets.len() == 1;

        for (idx, target) in targets.iter().enumerate() {
            let (sender, receiver) = channel::<Output>();

            // Start UI on its own thread b/c `Vmtest::run_one()` will block
            let name = target.name.clone();
            let ui = thread::spawn(move || Self::target_ui(receiver, name, show_cmd));

            // Run a target
            self.vmtest.run_one(idx, sender);

            let rc = ui
                .join()
                .expect("Failed to join UI thread")
                // Transform an error into a pre-baked error code that represent a failure to run the VM.
                .unwrap_or(EX_UNAVAILABLE);

            if single_cmd {
                return rc;
            }

            failed += match rc {
                0 => 0,
                _ => 1,
            }
        }

        failed
    }
}
