//! va — a video editing CLI for agents.
mod cache;
mod caption;
mod cli;
mod ffmpeg;
mod frames;
mod graph;
mod media;
mod paths;
mod project;
mod render;
mod report;
mod time;

fn main() {
    cli::run();
}
