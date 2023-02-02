use crate::qemu::Output;
use crate::vmtest::Vmtest;

struct Ui {}

impl Ui {
    /// Construct a new UI
    pub fn new(vmtest: Vmtest) -> Self {
        Self {}
    }

    /// Run all the targets in the provided `vmtest`
    ///
    /// Note this function is "infallible" b/c on error it will display
    /// the appropriate error message to screen.
    pub fn run() {}
}
