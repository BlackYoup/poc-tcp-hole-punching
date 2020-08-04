#[macro_use] extern crate log;

mod raw_tcp;
mod raw_yamux;

fn main() {
  pretty_env_logger::init_timed();
  let mut args = std::env::args();
  args.next();
  let t = args.next().unwrap();

  match t.as_ref() {
    "raw-tcp" => raw_tcp::raw_tcp().unwrap(),
    "yamux" => raw_yamux::raw_yamux(),
    _ => eprintln!("Error, unknown mode")
  };
}