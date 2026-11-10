//! splice — a video editing CLI for agents.
mod audio;
mod cache;
mod caption;
mod cli;
mod ffmpeg;
mod frames;
mod graph;
mod history;
mod index;
mod media;
mod paths;
mod project;
mod render;
mod report;
mod time;
mod transcript;

fn main() {
    cli::run();
}
