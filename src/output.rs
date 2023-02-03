use anyhow::Result;

/// This enum encapsulates real time updates about the VM.
///
/// This is essentially a state machine where on a successful VM
/// run the receiver should expect to see at least one of each variant
/// in order as defined.
///
/// Failure is defined as seeing an `Err` in one of the `*End` variants.
/// Receivers should treat failures as terminal and not expect any more
/// updates.
pub enum Output {
    /// VM boot begins
    BootStart,
    /// Output related to VM boot
    Boot(String),
    /// Boot finished with provided with provided result
    BootEnd(Result<()>),

    /// Starting to wait for QGA
    WaitStart,
    /// QGA waiting finished with provided result
    WaitEnd(Result<()>),

    /// Setting up VM has begun
    SetupStart,
    /// Output related to setting up the VM
    Setup(String),
    /// Setting up VM finished with provided result
    SetupEnd(Result<()>),

    /// Starting to run command
    CommandStart,
    /// Output related to running the target command
    Command(String),
    /// Command finished with provided exit code
    CommandEnd(Result<i64>),
}
