//! Print the system prompt Lucy would send, using your config and persona folder.
//!
//! ```sh
//! cargo run -q -p lucy-core --example system_prompt            # full voice
//! cargo run -q -p lucy-core --example system_prompt -- --plain # plain mode
//! ```

use lucy_core::config::{self, Config};
use lucy_core::persona::{self, Personas, PromptContext, build_system_prompt};
use lucy_core::sessions::Origin;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let plain = std::env::args().any(|arg| arg == "--plain");
    let config = match config::default_path() {
        Some(path) => Config::load(&path)?,
        None => None,
    };

    let persona_name = config
        .as_ref()
        .map_or("lucy", |c| c.defaults.persona.as_str());
    let persona = Personas::new(persona::default_user_dir()).load(persona_name)?;
    let origin = config.as_ref().map(|c| Origin {
        profile: c.defaults.profile.clone(),
        model: c.profiles[&c.defaults.profile].model.clone(),
    });

    let ctx = PromptContext {
        plain,
        now: chrono::Local::now().fixed_offset(),
        machine: Some(persona::detect_machine()),
        user_name: config.as_ref().and_then(|c| c.user.name.clone()),
        origin,
    };
    println!("{}", build_system_prompt(&persona, &ctx));
    Ok(())
}
