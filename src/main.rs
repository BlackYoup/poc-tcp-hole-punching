use std::net::{TcpListener, TcpStream};
use std::io;
use std::io::{Read, Write};
use std::mem;
use std::os::unix::io::AsRawFd;
use net2::TcpBuilder;
use net2::unix::UnixTcpBuilderExt;

#[derive(Eq, PartialEq)]
enum State {
  SEND_PUBLIC,
  WAITING_PUNCH,
}

struct Stream {
  stream: TcpStream,
  state: State
}

fn hole_punching(mut client_a: Stream, mut client_b: Stream) -> Result<(), io::Error> {
  let client_a_peer = client_a.stream.peer_addr().unwrap();
  let client_b_peer = client_b.stream.peer_addr().unwrap();
  let client_a_punch_order = format!("PUNCH {} {}", client_b_peer.ip(), client_b_peer.port());
  let client_b_punch_order = format!("PUNCH {} {}", client_a_peer.ip(), client_a_peer.port());

  client_a.stream.write_all(client_a_punch_order.as_bytes())?;
  client_a.stream.flush()?;
  client_b.stream.write_all(client_b_punch_order.as_bytes())?;
  client_b.stream.flush()?;

  Ok(())
}

fn client_punch(stream: TcpStream, addr: String, port: String) -> Result<(), io::Error> {
  println!("Should punch {}:{}", addr, port);
  let local_addr = stream.local_addr().unwrap();
  let tcp = TcpBuilder::new_v4()?;
  tcp.reuse_address(true)?.reuse_port(true)?;
  let local_bind = tcp.bind(local_addr)?.listen(1024)?;
  local_bind.set_nonblocking(true)?;

  let mut incoming_remote = false;

  loop {
    for remote_stream in local_bind.incoming() {
      match remote_stream {
        Ok(rs) => {
          incoming_remote = true;
          println!("Got remote stream incoming from: {}:{}", rs.peer_addr()?.ip(), rs.peer_addr()?.port());
          break;
        },
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
        Err(e) => panic!("error: {}", e)
      }
    }

    if incoming_remote == true {
      break;
    }

    println!("Trying to connect to remote {}:{}", addr, port);
    match TcpStream::connect(format!("{}:{}", addr, port)) {
      Ok(mut remote_connect) => {
        remote_connect.write_all(b"hello?")?;
        remote_connect.flush()?;
      }
      Err(ref e) if e.kind() == io::ErrorKind::ConnectionRefused => continue,
      Err(e) => panic!("error: {}", e)
    }
  }

  Ok(())
}

fn send_public(mut stream: &TcpStream) -> Result<(), io::Error> {
  println!("send public");
  let peer_addr = stream.peer_addr()?;
  let body: String = format!("PUBLIC {}:{}", peer_addr.ip(), peer_addr.port());
  println!("Writing to client {}:{}", peer_addr.ip(), peer_addr.port());
  let mut buffer = body.as_bytes();
  stream.write_all(&mut buffer)?;
  stream.flush()?;

  Ok(())
}

fn server() -> Result<(), io::Error> {
  println!("Starting in server mode");

  let listener = TcpListener::bind("0.0.0.0:4040")?;
  listener.set_nonblocking(true)?;
  let mut streams: Vec<Stream> = Vec::new();

  loop {
    for stream in listener.incoming() {
      match stream {
        Ok(s) => {
          println!("Accepting new stream");
          streams.push(Stream {
            stream: s,
            state: State::SEND_PUBLIC
          });
        },
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
        Err(e) => panic!("error: {}", e)
      }
    }

    for stream in &mut streams {
      match stream.state {
        State::SEND_PUBLIC => {
          send_public(&stream.stream)?;
          stream.state = State::WAITING_PUNCH;
        },
        _ => {}
      }
    }

    if streams.len() == 2 && streams.iter().find(|s| s.state == State::SEND_PUBLIC).is_none() {
      println!("Got two stream, should punch each other");
      let client_a = streams.remove(0);
      let client_b = streams.remove(0);
      hole_punching(client_a, client_b);
    }

    std::thread::sleep(std::time::Duration::from_millis(10));
  }
}

fn client(addr: String, port: String) -> Result<(), io::Error> {
  println!("Starting in client mode");

  let connect = format!("{}:{}", addr, port);
  let tcp = TcpBuilder::new_v4()?;
  tcp.reuse_address(true)?.reuse_port(true).unwrap();
  let mut stream = tcp.connect(connect)?;
  let mut buffer = [0; 128].to_vec();
  loop {
    match stream.read(&mut buffer) {
      Ok(0) => continue,
      Ok(_) => break,
      Err(e) => panic!("error: {}", e)
    }
  }

  println!("Received: {}", String::from_utf8(buffer).unwrap());

  println!("Waiting for hole punching order");

  let mut hole_punching_buffer = String::new();
  stream.read_to_string(&mut hole_punching_buffer)?;

  let mut splitted = hole_punching_buffer.split(" ");

  match splitted.next() {
    Some("PUNCH") => client_punch(stream, splitted.next().unwrap().to_string(), splitted.next().unwrap().to_string())?,
    Some("hello?") => println!("Other client talked to us??"),
    _ => panic!("Unknown order: {}", hole_punching_buffer)
  }

  Ok(())
}

fn main() -> Result<(), io::Error> {
  let mut args = std::env::args();
  args.next();
  let t = args.next().unwrap();

  if t == "CLIENT" {
    client(args.next().unwrap(), args.next().unwrap())?;
  } else if t == "SERVER" {
    server()?;
  } else {
    panic!("Unknown");
  }

  Ok(())
}
