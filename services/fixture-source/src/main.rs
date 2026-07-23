fn main() -> Result<(), Box<dyn std::error::Error>> {
    let observation = fixture_source::phase0_observation()?;
    println!("{}", String::from_utf8(observation.payload)?);
    Ok(())
}
