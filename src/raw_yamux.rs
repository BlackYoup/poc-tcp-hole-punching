use futures::{channel::mpsc, prelude::*};
use std::{net::{Ipv4Addr, SocketAddr, SocketAddrV4}, sync::Arc};
use tokio::{net::{TcpStream, TcpListener}, task};
use tokio_util::compat::{Compat, Tokio02AsyncReadCompatExt};
use yamux::{Config, Connection, Mode};
use tokio::runtime;
use futures_util::stream::TryStreamExt;
use net2::unix::UnixTcpBuilderExt;

fn server() {
  let mut runtime = runtime::Builder::new()
  .threaded_scheduler()
  .enable_io()
  .build()
  .unwrap();

  struct StreamInfo {
    stream: Compat<TcpStream>,
    addr: SocketAddr
  }

  let server = async move {
    let mut listener = TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 4040))).await.expect("bind");
    println!("Listening on port 4040");
    let mut streams: Vec<StreamInfo> = Vec::new();
    loop {
      let tcpstream = listener.accept().await.expect("accept");
      let peer = tcpstream.1;
      println!("Peer connected from {:?}", peer);
      let socket = tcpstream.0.compat();
        //.try_for_each_concurrent(None, |mut stream| async move {
          //let mut len = [0; 4];
          //stream.read_exact(&mut len).await?;
          //let mut buf = vec![0; u32::from_be_bytes(len) as usize];
          //stream.read_exact(&mut buf).await?;
          //stream.write_all(&buf).await?;
          //stream.close().await?;
          //Ok(())
      //})
      //.await
      //.expect("server works");
      streams.push(StreamInfo {
        stream: socket,
        addr: peer
      });

      if streams.len() == 2 {
        let stream1: StreamInfo = streams.remove(0);
        let stream2: StreamInfo = streams.remove(0);

        let message1_str = format!("{}:{}", stream2.addr.ip(), stream2.addr.port());
        let message1 = message1_str.as_bytes();

        let message2_str = format!("{}:{}", stream1.addr.ip(), stream1.addr.port());
        let message2 = message2_str.as_bytes();

        println!("\n{}\n{}", message1_str, message2_str);

        yamux::into_stream(Connection::new(stream1.stream, Config::default(), Mode::Server))
          .try_for_each_concurrent(None, |mut stream| async move {
            stream.write_all(message1).await?;
            stream.close().await?;
            Ok(())
          }).await.unwrap();

        yamux::into_stream(Connection::new(stream2.stream, Config::default(), Mode::Server))
          .try_for_each_concurrent(None, |mut stream| async move {
            stream.write_all(message2).await?;
            stream.close().await?;
            Ok(())
          }).await.unwrap();
      }
    }
  };

  runtime.block_on(server);
}

fn client(address: SocketAddr) {
    let mut runtime = runtime::Builder::new()
    .threaded_scheduler()
    .enable_io()
    .build()
    .unwrap();

    let client = async move {
      let tcp = net2::TcpBuilder::new_v4().unwrap();
      tcp.reuse_address(true).unwrap().reuse_port(true).unwrap();
      let stream = TcpStream::from_std(tcp.connect(&address).unwrap()).unwrap();
      let local_addr = stream.local_addr().unwrap();
      let socket = stream.compat();
      println!("Connected, local addr: {:?}", local_addr);
      let conn = Connection::new(socket, Config::default(), Mode::Client);
      let mut ctrl = conn.control();
      task::spawn(yamux::into_stream(conn).for_each(|_| future::ready(())));

      let data = "Hello";
      let mut ctrl2 = ctrl.clone();
      let other_peer = task::spawn(async move {
        let mut stream = ctrl2.open_stream().await?;
        stream.write_all(&(data.len() as u32).to_be_bytes()[..]).await?;
        stream.write_all(&data.as_bytes()).await?;
        let mut buffer = [0; 1024];
        let read = stream.read(&mut buffer).await?;
        let to_punch_hole = String::from_utf8_lossy(&buffer[..read]).to_string();
        println!("Read {:?}", to_punch_hole);
        Ok::<String, yamux::ConnectionError>(to_punch_hole)
      }).await.unwrap().unwrap();
      ctrl.close().await.expect("close connection");
      drop(ctrl);

      println!("Init tcp hole punch");

      let tcp = net2::TcpBuilder::new_v4().unwrap();
      tcp.reuse_address(true).unwrap().reuse_port(true).unwrap();
      let local_bind = tcp.bind(local_addr).unwrap();

      let mut listener = TcpListener::from_std(local_bind.clone().listen(1024).unwrap()).unwrap();
      task::spawn(async move {
        let stream = listener.accept().await.unwrap();
        println!("Accepted incoming from {:?}", stream.1);
        let conn = Connection::new(stream.0.compat(), Config::default(), Mode::Server);

        let data = "Hello world!";
        yamux::into_stream(conn).try_for_each_concurrent(None, |mut stream| async move {
          let mut buffer = [0; 1024];
          let read = stream.read(&mut buffer).await?;
          println!("Read from other peer: {:?}", String::from_utf8_lossy(&buffer[..read]).to_string());
          stream.write_all(&data.as_bytes()).await?;
          stream.flush().await.unwrap();
          Ok(())
        }).await.unwrap();
        println!("We are done!");
        std::process::exit(0);
      });

      loop {
        let tcp = net2::TcpBuilder::new_v4().unwrap();
        tcp.reuse_address(true).unwrap().reuse_port(true).unwrap();
        let local_bind = tcp.bind(local_addr).unwrap();

        let connect = local_bind.connect(other_peer.parse::<SocketAddr>().unwrap());
        match connect {
          Ok(stream) => {
            println!("We are connected! Try to init yamux :)");
            let stream = TcpStream::from_std(stream).unwrap().compat();
            let conn = Connection::new(stream, Config::default(), Mode::Client);
            let mut ctrl = conn.control();
            task::spawn(yamux::into_stream(conn).for_each(|_| future::ready(())));

            let data = "Hello world!";
            let mut ctrl2 = ctrl.clone();
            task::spawn(async move {
              let mut stream = ctrl2.open_stream().await?;
              stream.write_all(&data.as_bytes()).await?;
              stream.flush().await.unwrap();
              let mut buffer = [0; 1024];
              let read = stream.read(&mut buffer).await?;
              println!("Read from other peer: {:?}", String::from_utf8_lossy(&buffer[..read]).to_string());
              Ok::<(), yamux::ConnectionError>(())
            }).await.unwrap().unwrap();
            ctrl.close().await.expect("close connection");
            println!("We are done!");
            std::process::exit(0);
          },
          Err(e) => continue
        }
      }

    };

    println!("{:?}", runtime.block_on(client));
}

pub fn raw_yamux() {
  let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 4040));
  let mut mode = std::env::args();
  mode.next();
  mode.next();
  let mode = mode.next().unwrap();

  match mode.as_str() {
    "CLIENT" => client(addr),
    "SERVER" => server(),
    _ => panic!("unknown mode")
  };
}