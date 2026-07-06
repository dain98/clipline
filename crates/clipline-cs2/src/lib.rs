//! CS2 event source: a local HTTP endpoint receiving Valve Game State
//! Integration POSTs, and a delta tracker normalizing them into
//! `clipline_events::GameEvent`s. Anti-cheat-safe by construction — GSI is
//! Valve's official, officially-configured push feed; nothing touches the
//! game process.

pub mod listener;
pub mod payload;
pub mod tracker;

pub use listener::{bind, GsiSource};
pub use payload::GsiPayload;
pub use tracker::{GsiTracker, GsiUpdate};

/// Default loopback endpoint the shipped GSI config points at.
pub const DEFAULT_GSI_ADDR: &str = "127.0.0.1:27893";

/// Auth token in the shipped GSI config. Constant for now; per-install
/// tokens arrive with the setup UX milestone.
pub const DEFAULT_GSI_TOKEN: &str = "clipline-gsi-1";

/// The `gamestate_integration_clipline.cfg` contents matching the defaults
/// above. Written into `game/csgo/cfg` by the setup flow (or by hand).
pub fn gsi_config_template() -> String {
    format!(
        r#""Clipline GSI"
{{
	"uri"	"http://{DEFAULT_GSI_ADDR}/"
	"timeout"	"0.5"
	"buffer"	"0.1"
	"throttle"	"0.1"
	"heartbeat"	"30"
	"auth"
	{{
		"token"	"{DEFAULT_GSI_TOKEN}"
	}}
	"data"
	{{
		"provider"	"1"
		"map"	"1"
		"round"	"1"
		"player_id"	"1"
		"player_state"	"1"
		"player_match_stats"	"1"
	}}
}}
"#
    )
}
