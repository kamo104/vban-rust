use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::thread::{self};
use std::time::Duration;

use clap::{Parser, Subcommand};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Host};

#[derive(Debug, clap::Args)]
struct ReceiverArgs {
    /// The output audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    output_device: String,

    /// The address to bind the UDP socket to
    #[arg(long)]
    bind_address: SocketAddr,
}

#[derive(Debug, clap::Args)]
struct TransmitterArgs {
    /// The input audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    input_device: String,

    /// The target to send audio data to
    #[arg(long)]
    target: SocketAddr,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Receiver(ReceiverArgs),
    Transmitter(TransmitterArgs),
}

#[derive(Debug, clap::Args)]
struct GlobalArgs {
    /// Whether to list available input devices
    #[arg(long, default_value_t = false)]
    list_inputs: bool,

    /// Whether to list available output devices
    #[arg(long, default_value_t = false)]
    list_outputs: bool,

    /// Whether to list supported configs
    #[arg(long, default_value_t = false)]
    list_configs: bool,

    /// The delay in milliseconds
    #[arg(short, long, default_value_t = 10.0)]
    latency: f32,
}

#[derive(Parser, Debug)]
#[command(author, version, about = "cross-platform vban", long_about = None)]
struct Args {
    #[clap(flatten)]
    global_args: GlobalArgs,

    #[clap(subcommand)]
    command: Option<Commands>,
}

fn receiver(
    host: &Host,
    global_args: GlobalArgs,
    receiver_args: ReceiverArgs,
) -> anyhow::Result<()> {
    let output_device = if receiver_args.output_device == "default" {
        host.default_output_device()
    } else {
        host.input_devices()?.find(|x| {
            x.name()
                .map(|y| y == receiver_args.output_device)
                .unwrap_or(false)
        })
    }
    .expect("Failed to find output device");

    println!("Using \"{}\" output device.", output_device.name().unwrap());
    if global_args.list_configs {
        println!("Supported configs:");
        output_device
            .supported_output_configs()
            .unwrap()
            .for_each(|x| println!("\t{:?}", x));
        return Ok(());
    }

    let mut config = output_device.default_output_config().unwrap();
    config.sample_format();

    let socket = UdpSocket::bind(&receiver_args.bind_address).unwrap();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            if let Ok((amt, _)) = socket.recv_from(&mut buffer) {
                let samples: Vec<f32> = buffer[..amt]
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                let _ = tx.send(samples);
            }
            // println!("New packet!");
        }
    });

    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        if let Ok(samples) = rx.try_recv() {
            for (d, s) in data.iter_mut().zip(samples.iter()) {
                *d = *s;
            }
            // println!("Output some real data");
        } else {
            data.fill(0.0);
            // println!("Output some FAKE data");
        }
    };

    let output_stream = output_device.build_output_stream(
        &config.into(),
        output_data_fn,
        |err| eprintln!("Stream error: {}", err),
        None,
    )?;
    output_stream.play()?;

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn transmitter(host: &Host, global_args: GlobalArgs, args: TransmitterArgs) -> anyhow::Result<()> {
    let input_device = if args.input_device == "default" {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|x| x.name().map(|y| y == args.input_device).unwrap_or(false))
    }
    .expect("Failed to find input device");

    println!("Using \"{}\" input device.", input_device.name().unwrap());

    if global_args.list_configs {
        println!("Supported configs:");
        input_device
            .supported_input_configs()
            .unwrap()
            .for_each(|x| println!("\t{:?}", x));
        return Ok(());
    }

    let mut config = input_device.default_input_config().unwrap();

    let (tx, rx) = mpsc::channel();

    let nb_channels = config.channels();
    let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        let mut stereo_data = Vec::with_capacity(data.len() * 2);

        // If the input is mono (1 channel), duplicate each sample
        if nb_channels == 1 {
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
        &config.into(),
        input_data_fn,
        |err| eprintln!("An error occurred on stream: {}", err),
        None,
    )?;
    input_stream.play()?;

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(args.target)?;

    thread::spawn(move || {
        loop {
            if let Ok(buffer) = rx.recv() {
                let packet: Vec<u8> = buffer.iter().flat_map(|s| s.to_le_bytes()).collect();
                let _ = socket.send(&packet);
            }
        }
    });

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
// TODO: use rust rubato for converting between sample rates
// https://github.com/HEnquist/rubato
// TODO: handle different different types of samples(i24,i32,f32)
// https://github.com/RustAudio/cpal/blob/master/examples/beep.rs
// TODO: handle different amounts of channels
// TODO: parse the vban network stream config
// https://vb-audio.com/Voicemeeter/VBANProtocol_Specifications.pdf
// TODO: use the VBAN header in network communication.
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let global_args = args.global_args;
    let host = cpal::default_host();

    if global_args.list_inputs {
        println!("Available input devices:");
        let print_fn = |device: &Device| match global_args.list_configs {
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
    if global_args.list_outputs {
        println!("Available output devices:");
        let print_fn = |device: &Device| match global_args.list_configs {
            true => {
                let configs: Vec<String> = device
                    .supported_output_configs()
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
        host.output_devices()
            .unwrap()
            .for_each(|dev| print_fn(&dev));
        return Ok(());
    }

    let command = match args.command {
        Some(cmd) => cmd,
        None => return Ok(()),
    };
    return match command {
        Commands::Receiver(receiver_args) => receiver(&host, global_args, receiver_args),
        Commands::Transmitter(transmitter_args) => {
            transmitter(&host, global_args, transmitter_args)
        }
    };
}
