//! The `flint` command line: a thin shell over [`flint_app_service`]. It collects arguments and
//! passwords, calls the service, and prints what happened.

mod cli;
mod input;
mod output;
mod run;

pub use cli::{Cli, Command, RemoteArgs};
pub use input::{Input, StdinInput, TerminalInput};
pub use run::{Env, Error, Result, run};
