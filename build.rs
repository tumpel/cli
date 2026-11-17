#[path = "release_version.rs"]
mod release_version;

fn main() -> Result<(), String> {
    release_version::release_version()
}
