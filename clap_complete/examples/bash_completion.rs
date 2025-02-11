use clap::App;
use clap_complete::{generate, shells::Bash};
use std::io;

fn main() {
    let mut app = App::new("myapp")
        .subcommand(App::new("test").subcommand(App::new("config")))
        .subcommand(App::new("hello"));

    generate(Bash, &mut app, "myapp", &mut io::stdout());
}
