use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

const SOCKET_PATH: &str = "/tmp/waybar-hovermenu.sock";

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: hovermenu-ctl <command> [module]");
        eprintln!("Commands: follow, status, hover, leave, click, action");
        std::process::exit(1);
    }

    let command = &args[1];
    let module = args.get(2).map(|s| s.as_str()).unwrap_or("");

    // Build the command string
    let cmd = if module.is_empty() {
        format!("{}\n", command)
    } else {
        format!("{} {}\n", command, module)
    };

    // Connect to the daemon
    let mut stream = match UnixStream::connect(SOCKET_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to daemon: {}", e);
            eprintln!("Is waybar-hovermenu running?");
            std::process::exit(1);
        }
    };

    // Send the command
    if let Err(e) = stream.write_all(cmd.as_bytes()) {
        eprintln!("Failed to send command: {}", e);
        std::process::exit(1);
    }

    // For follow command, keep reading and printing output
    // For other commands, just read one line (if any)
    if command == "follow" || command == "status" {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            match line {
                Ok(line) => println!("{}", line),
                Err(_) => break,
            }

            // For status, just print one line
            if command == "status" {
                break;
            }
        }
    }
}
