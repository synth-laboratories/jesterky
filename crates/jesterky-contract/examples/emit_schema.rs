use jesterky_contract::{manifest_schema_json, workflow_schema_json};

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("workflow") => print!("{}", workflow_schema_json()),
        Some("manifest") => print!("{}", manifest_schema_json()),
        Some(other) => {
            eprintln!("unknown schema `{other}`; expected `workflow` or `manifest`");
            std::process::exit(2);
        }
        None => {
            println!("{}", workflow_schema_json());
            print!("{}", manifest_schema_json());
        }
    }
}
