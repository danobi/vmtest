use std::sync::mpsc::{channel, Receiver};
use std::thread;

use console::Term;

use crate::qemu::Output;
use crate::vmtest::Vmtest;

/// Console UI
///
/// This struct handles all the fancy pretty printing as well as
/// formatting and reporting any errors.
pub struct Ui {
    vmtest: Vmtest,
}

impl Ui {
    /// Construct a new UI
    pub fn new(vmtest: Vmtest) -> Self {
        Self { vmtest }
    }

    /// UI for a single target. Must be run on its own thread.
    fn target_ui(_term: Term, _updates: Receiver<Output>, _target: String) {
        unimplemented!();
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

            // Run a taget
            self.vmtest
                .run_one(idx, sender)
                .expect("XXX make this infallible");

            // UI thread does not return errors; they get printed to console
            let _ = ui.join();
        }
    }
}
