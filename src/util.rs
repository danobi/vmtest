use std::env::temp_dir;
use std::path::PathBuf;

use rand::Rng as _;


/// Generate a path to a randomly named socket
pub(crate) fn gen_sock(prefix: &str) -> PathBuf {
  let id = rand::thread_rng().gen_range(100_000..1_000_000);
  temp_dir().join(format!("{prefix}-{id}.sock"))
}
