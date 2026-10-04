use std::io::{self, BufRead};

/// Where secrets come from.
pub trait Input {
    /// Reads one secret (never echoed when it comes from a terminal).
    fn secret(&mut self, prompt: &str) -> io::Result<String>;
}

/// Prompts on the terminal without echoing.
pub struct TerminalInput;

impl Input for TerminalInput {
    fn secret(&mut self, prompt: &str) -> io::Result<String> {
        rpassword::prompt_password(format!("{prompt}: "))
    }
}

/// Reads secrets from standard input, one per line.
pub struct StdinInput<R>(pub R);

impl<R: BufRead> Input for StdinInput<R> {
    fn secret(&mut self, prompt: &str) -> io::Result<String> {
        let mut line = String::new();
        if self.0.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("standard input ended before {prompt}"),
            ));
        }
        Ok(line.trim_end_matches(['\n', '\r']).to_owned())
    }
}
