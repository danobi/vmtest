use crate::vmtest::Vmtest;

/// Console UI
///
/// This struct handles all the fancy pretty printing as well as
/// formatting and reporting any errors.
pub struct Ui {
    _vmtest: Vmtest,
}

impl Ui {
    /// Construct a new UI
    pub fn new(vmtest: Vmtest) -> Self {
        Self { _vmtest: vmtest }
    }

    /// Run all the targets in the provided `vmtest`
    ///
    /// Note this function is "infallible" b/c on error it will display
    /// the appropriate error message to screen.
    pub fn run(self) {}
}
