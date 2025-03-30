use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::thread::{self, sleep};
use std::time::Duration;

use clap::Parser;
use cpal::Device;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

#[derive(Parser, Debug)]
#[command(version, about = "audio to udp", long_about = None)]
struct Opt {
    /// Whether to list available input devices
    #[arg(long, default_value_t = false)]
    list_inputs: bool,

    /// Whether to list supported configs
    #[arg(long, default_value_t = false)]
    supported_configs: bool,

    /// The input audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    input_device: String,

    /// The delay before sending
    // #[arg(short, long, default_value_t = 10.0)]
    // latency: f32,

    /// The target to send audio data to
    #[arg(long)]
    target: SocketAddr,
}

// TODO: handle different amounts of channels and sample rates
fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    let host = cpal::default_host();

    if opt.list_inputs {
        println!("Available input devices:");
        let print_fn = |device: &Device| match opt.supported_configs {
            true => {
                let configs: Vec<String> = device
                    .supported_input_configs()
                    .unwrap()
                    .into_iter()
                    .map(|x| format!("{:?}", x))
                    .collect();
                println!(
                    "\t\"{}\"\n\t\t{}",
                    device.name().unwrap(),
                    configs.join(",\n\t\t")
                )
            }
            false => println!("\t{}", device.name().unwrap()),
        };
        host.input_devices().unwrap().for_each(|dev| print_fn(&dev));
        return Ok(());
    }

    let input_device = if opt.input_device == "default" {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|x| x.name().map(|y| y == opt.input_device).unwrap_or(false))
    }
    .expect("Failed to find input device");

    if opt.supported_configs {
        println!("Supported configs:");
        input_device
            .supported_input_configs()
            .unwrap()
            .for_each(|x| println!("\t{:?}", x));
        return Ok(());
    }

    let config: cpal::StreamConfig = input_device.default_input_config()?.into();

    let (tx, rx) = mpsc::channel();

    let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        let mut stereo_data = Vec::with_capacity(data.len() * 2);

        // If the input is mono (1 channel), duplicate each sample
        if config.channels == 1 {
            for &sample in data {
                stereo_data.push(sample);
                stereo_data.push(sample);
            }
            let _ = tx.send(stereo_data);
        } else {
            let _ = tx.send(data.to_vec());
        }
    };

    let input_stream = input_device.build_input_stream(
        &config,
        input_data_fn,
        |err| eprintln!("An error occurred on stream: {}", err),
        None,
    )?;
    input_stream.play()?;

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(opt.target)?;

    thread::spawn(move || {
        loop {
            if let Ok(buffer) = rx.recv() {
                let packet: Vec<u8> = buffer.iter().flat_map(|s| s.to_ne_bytes()).collect();
                let _ = socket.send(&packet);
            }
        }
    });

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
