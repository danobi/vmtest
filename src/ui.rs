use std::cmp::min;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use anyhow::{anyhow, Error};
use console::{style, Term};

use crate::output::Output;
use crate::vmtest::Vmtest;

const WINDOW_LENGTH: usize = 5;

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

impl Stage {
    /// Start a new stage.
    ///
    /// We take ownership of the previous stage to control the drop order.
    /// Without this, something like:
    ///
    ///         stage = Some(Stage::new(..));
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
    fn window_size(&self) -> usize {
        min(self.lines.len(), WINDOW_LENGTH)
    }

    /// Add a line to the output window.
    ///
    /// If over the window size, older text will be cleared from the screen.
    ///
    /// Note we never expect printing to terminal to fail. Even if it did,
    /// we'd have no choice but to panic, so panic.
    fn print_line(&mut self, line: &str) {
        // Clear previously visible lines
        self.term.clear_last_lines(self.window_size()).unwrap();

        // Compute new visible lines
        self.lines.push(line.to_string());
        // Unwrap should never fail b/c we're sizing with `min()`
        let window = self.lines.windows(self.window_size()).last().unwrap();

        // Print visible lines
        for line in window {
            self.term
                .write_line(&format!("{}", style(line).dim()))
                .unwrap();
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
        self.term
            .clear_last_lines(self.window_size())
            .expect("Failed to clear terminal");

        if self.expand {
            for line in &self.lines {
                self.term
                    .write_line(&format!("{}", style(line).dim()))
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
    stage.print_line(&format!("{}", style(&err).red().bright()));
    stage.expand(true);
}

impl Ui {
    /// Construct a new UI
    pub fn new(vmtest: Vmtest) -> Self {
        Self { vmtest }
    }

    /// UI for a single target. Must be run on its own thread.
    fn target_ui(term: Term, updates: Receiver<Output>, target: String) {
        let mut stage = Stage::new(term.clone(), &heading(&target, 1), None);
        let mut stages = 0;
        let mut errors = 0;

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
                Output::Boot(s) => stage.print_line(s),
                Output::BootEnd(r) => {
                    if let Err(e) = r {
                        error_out_stage(&mut stage, e);
                        errors += 1;
                    }
                }
                Output::WaitStart => {
                    stage = Stage::new(
                        term.clone(),
                        &heading("Waiting on userspace", 2),
                        Some(stage),
                    );
                    stages += 1;
                }
                Output::WaitEnd(r) => {
                    if let Err(e) = r {
                        error_out_stage(&mut stage, e);
                        errors += 1;
                    }
                }
                Output::SetupStart => {
                    stage = Stage::new(term.clone(), &heading("Setting up VM", 2), Some(stage));
                    stages += 1;
                }
                Output::Setup(s) => stage.print_line(s),
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
                Output::Command(s) => stage.print_line(s),
                Output::CommandEnd(r) => match r {
                    Ok(retval) => {
                        if *retval != 0 {
                            error_out_stage(
                                &mut stage,
                                &anyhow!("Command failed with exit code: {}", retval),
                            );
                            errors += 1;
                        }
                    }
                    Err(e) => {
                        error_out_stage(&mut stage, e);
                        errors += 1;
                    }
                },
            }
        }

        // Only clear target stages if target was successful
        if errors == 0 {
            term.clear_last_lines(stages).unwrap();
        }
    }

    /// Run all the targets in the provided `vmtest`
    ///
    /// Note this function is "infallible" b/c on error it will display
    /// the appropriate error message to screen.
    pub fn run(self) {
        let term = Term::stdout();
        for (idx, target) in self.vmtest.targets().iter().enumerate() {
            let (sender, receiver) = channel::<Output>();

            // Start UI on its own thread b/c `Vmtest::run_one()` will block
            let name = target.name.clone();
            let term_clone = term.clone();
            let ui = thread::spawn(move || Self::target_ui(term_clone, receiver, name));

            // Run a target
            self.vmtest.run_one(idx, sender);

            // UI thread does not return errors; they get printed to console
            let _ = ui.join();
        }
    }
}
