//! Command implementations

pub mod curate;

pub mod doctor;

pub mod extract;

pub mod info;

pub mod render;

/// Print the colophon ASCII banner to stderr (bold slate).
/// Suppressed when stderr is not a terminal (piped/redirected).
pub fn banner() {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    eprint!(
        "\x1b[1;38;2;123;134;153m\
\n┌─┐┌─┐┬  ┌─┐┌─┐┬ ┬┌─┐┌┐┌\
\n│  │ ││  │ │├─┘├─┤│ ││││\
\n└─┘└─┘┴─┘└─┘┴  ┴ ┴└─┘┘└┘\
\n\x1b[0m\n"
    );
}
