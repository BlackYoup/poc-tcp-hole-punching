use std::net::{TcpListener, TcpStream};
use std::io;
use std::io::{Read, Write};
use std::mem;
use std::os::unix::io::AsRawFd;
use net2::TcpBuilder;
use net2::unix::UnixTcpBuilderExt;
use bytes::{BufMut, BytesMut};

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

  let client_a_addr = format!("{} {}", client_a_peer.ip(), client_a_peer.port());
  let client_b_addr = format!("{} {}", client_b_peer.ip(), client_b_peer.port());

  let mut client_a_punch_order = BytesMut::new();
  client_a_punch_order.put_u8(1);
  client_a_punch_order.put_u8(client_b_addr.len() as _);
  client_a_punch_order.put(client_b_addr.as_bytes());

  let mut client_b_punch_order = BytesMut::new();
  client_b_punch_order.put_u8(1);
  client_b_punch_order.put_u8(client_a_addr.len() as _);
  client_b_punch_order.put(client_a_addr.as_bytes());

  client_a.stream.write_all(&client_a_punch_order)?;
  client_a.stream.flush()?;
  client_b.stream.write_all(&client_b_punch_order)?;
  client_b.stream.flush()?;

  Ok(())
}

fn client_punch(mut stream: TcpStream, addr: String, port: String) -> Result<(), io::Error> {
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

    let mut buff = [0; 128];
    match stream.read(&mut buff) {
      Ok(0) => {},
      Ok(sz) => println!("I read {} from original stream :o: {:?}", sz, &buff[..sz]),
      Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {},
      Err(e) => panic!("Unknown error: {}", e)
    }

    println!("Trying to connect to remote {}:{}", addr, port);
    let remote_tcp = TcpBuilder::new_v4()?;
    remote_tcp.reuse_address(true)?.reuse_port(true)?;
    remote_tcp.bind(format!("0.0.0.0:{}", local_addr.port()))?;

    match remote_tcp.connect(format!("{}:{}", addr, port)) {
      Ok(mut remote_connect) => {
        println!("Connected!");
        remote_connect.write_all(b"hello?")?;
        remote_connect.flush()?;
        let mut response = [0; 6];
        remote_connect.read_exact(&mut response)?;
        println!("Received response: {:?}", response);
        break;
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
  let addr = format!("{}:{}", peer_addr.ip(), peer_addr.port());
  let mut body = BytesMut::new();
  body.put_u8(0);
  body.put_u8(addr.len() as u8);
  body.put(addr.as_bytes());
  println!("Writing to client {}:{} -> {:?}", peer_addr.ip(), peer_addr.port(), body);
  stream.write_all(&body)?;
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
      hole_punching(client_a, client_b)?;
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
  let mut order = [0; 1];
  stream.read_exact(&mut order)?;

  if order[0] != 0 {
    panic!("Unexpected message: {}", order[0]);
  }

  let mut order_length = [0; 1];
  stream.read_exact(&mut order_length)?;
  let mut order_content = Vec::with_capacity(order_length[0] as usize);
  unsafe { order_content.set_len(order_length[0] as usize) };
  stream.read_exact(&mut order_content)?;

  println!("Received: {}, {}, {}", order[0], order_length[0], String::from_utf8(order_content).unwrap());

  println!("Waiting for hole punching order");

  let mut order = [0; 1];
  stream.read_exact(&mut order)?;
  match order[0] {
    1 => {
      let mut order_len = [0; 1];
      stream.read_exact(&mut order_len)?;
      let mut order_body = Vec::with_capacity(order_len[0] as usize);
      unsafe { order_body.set_len(order_len[0] as usize) };
      stream.read_exact(&mut order_body)?;
      let order_body_string = String::from_utf8(order_body).unwrap();
      let mut splitted = order_body_string.split(" ");

      client_punch(stream, splitted.next().unwrap().to_string(), splitted.next().unwrap().to_string())?
    }
    _ => panic!("Unknown order: {}", order[0])
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
