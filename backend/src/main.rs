use tokio::sync::Mutex;
use std::env;
use http::Method;
use tower_http::cors::{Any, CorsLayer};
use tower_sessions::{cookie::{time::Duration, SameSite}, Expiry, MemoryStore, SessionManagerLayer};
use tower_http::services::{ServeDir, ServeFile};
use std::sync::Arc;
use axum::{routing::{get, post}, Router};
use clap::{arg, command, value_parser};

mod config;
mod database;
mod shell;
mod api;
mod constants;
mod util;

use config::Config;
use shell::interactive_shell;
use database::Database;

struct AppState {
	config: Config,
	database: Option<Database>
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
	// TODO: args for database path, user files path, etc. ?

	let args = command!()
		.arg(
			arg!(--address <string> "The ip address the server listens on.")
				.required(false)
				.value_parser(value_parser!(String))
		)
		.arg(
			arg!(--port <number> "The port the server listens on.")
				.required(false)
				.value_parser(value_parser!(String))
		)
		.arg(
			arg!(--securecookies <boolean> "Whether session cookies should be secure or not.")
				.required(false)
				.value_parser(value_parser!(String))
		)
		.get_matches();

	// Print working directory
	let working_dir = env::current_dir()?;
	println!("Working directory: {}", working_dir.into_os_string().into_string().unwrap());

	// Get config
	let mut config = Config::initialise()?;

	// Override some config values if user supplied some arguments.
	if let Some(address) = args.get_one::<String>("address") {
		config.ip_address = address.clone();
	}

	if let Some(port) = args.get_one::<String>("port") {
		if let Ok(port) = port.trim().parse::<u16>() {
			config.port = port;
		} else {
			eprintln!("Failed to parse port provided in program arguments! Using config port of {} instead.", config.port);
		}
	}

	if let Some(secure) = args.get_one::<bool>("securecookies") {
		config.secure_cookies = *secure;
	}

	// Initialise missing directories defined in the config
	config.initialise_directories()?;

	// Open the database
	let database_instance = Some(Database::open(&config)?);
	
	// Create app state to be shared
	let config_clone = config.clone();

	let shared_app_state = Arc::new(Mutex::new(AppState {
		config: config,
		database: database_instance
	}));

	// Create the CORS layer
	let cors = CorsLayer::new()
		.allow_methods([ Method::GET, Method::POST ])
		.allow_origin(Any);

	// Create session store
	let session_store = MemoryStore::default();

	// Create session store layer
	let session_layer = SessionManagerLayer::new(session_store)
		.with_secure(config_clone.secure_cookies)
		.with_same_site(SameSite::Strict)
		.with_expiry(Expiry::OnInactivity(Duration::seconds(constants::SESSION_EXPIRY_TIME_SECONDS)))
		.with_signed(config_clone.session_secret_key);

	// Create router
	let router = Router::new()
		.route_service("/", ServeFile::new("frontend/dist/index.html"))
		.route_service("/assets", ServeDir::new("frontend/dist/assets"))

		// Account apis
		.route("/api/claimaccount", post(api::account::claim_account_api))
		.route("/api/checkclaimcode", post(api::account::check_claim_code_api))
		.route("/api/getusersalt", post(api::account::get_user_salt_api))
		.route("/api/getsessioninfo", get(api::account::get_session_info_api))
		.route("/api/logout", post(api::account::logout_api))
		.route("/api/login", post(api::account::login_api))

		// Filesystem apis
		.route("/api/getstorageused", get(api::filesystem::get_storage_used_api))
		.route("/api/getfilesystem", post(api::filesystem::get_filesystem_api))
		.route("/api/createfolder", post(api::filesystem::create_folder_api))

		// Transfer apis
		// TODO:

		.with_state(shared_app_state.clone())
		.layer(session_layer)
		.layer(cors);

	// Create listener
	let listener = tokio::net::TcpListener::bind(format!("{}:{}", config_clone.ip_address, config_clone.port)).await.unwrap();

	// Start server
	println!("Server listening on {}:{}", config_clone.ip_address, config_clone.port);

	axum::serve(listener, router)
		.with_graceful_shutdown(interactive_shell(shared_app_state.clone())) // Start the interactive shell
		.await
		.unwrap();

	// Close database
	println!("Closing database...");

	let database = shared_app_state.lock().await.database.take().expect("Database is none!");
	database.close();

	Ok(())
}
