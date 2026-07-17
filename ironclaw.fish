# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_ironclaw_global_optspecs
	string join \n cli-only no-db m/message= c/config= no-onboard h/help V/version
end

function __fish_ironclaw_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_ironclaw_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_ironclaw_using_subcommand
	set -l cmd (__fish_ironclaw_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c ironclaw -n "__fish_ironclaw_needs_command" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_needs_command" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_needs_command" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -s V -l version -d 'Print version'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "run" -d 'Run the agent (default if no subcommand given)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "onboard" -d 'Interactive onboarding wizard'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "config" -d 'Manage configuration settings'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "tool" -d 'Manage WASM tools'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "mcp" -d 'Manage MCP servers (hosted tool providers)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "memory" -d 'Query and manage workspace memory'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "pairing" -d 'DM pairing (approve inbound requests from unknown senders)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "service" -d 'Manage OS service (launchd / systemd)'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "doctor" -d 'Probe external dependencies and validate configuration'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "status" -d 'Show system health and diagnostics'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "completion" -d 'Generate shell completion scripts'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "worker" -d 'Run as a sandboxed worker inside a Docker container (internal use). This is invoked automatically by the orchestrator, not by users directly'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "claude-bridge" -d 'Run as a Claude Code bridge inside a Docker container (internal use). Spawns the `claude` CLI and streams output back to the orchestrator'
complete -c ironclaw -n "__fish_ironclaw_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand run" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -l skip-auth -d 'Skip authentication (use existing session)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -l channels-only -d 'Reconfigure channels only'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand onboard" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "init" -d 'Generate a default config.toml file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "list" -d 'List all settings and their current values'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "get" -d 'Get a specific setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "set" -d 'Set a setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "reset" -d 'Reset a setting to its default value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "path" -d 'Show the settings storage info'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and not __fish_seen_subcommand_from init list get set reset path help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -s o -l output -d 'Output path (default: ~/.ironclaw/config.toml)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -l force -d 'Overwrite existing file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from init" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -s f -l filter -d 'Show only settings matching this prefix (e.g., "agent", "heartbeat")' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from set" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from reset" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from path" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "init" -d 'Generate a default config.toml file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "list" -d 'List all settings and their current values'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "get" -d 'Get a specific setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "set" -d 'Set a setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "reset" -d 'Reset a setting to its default value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "path" -d 'Show the settings storage info'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "install" -d 'Install a WASM tool from source directory or .wasm file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "list" -d 'List installed tools'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "remove" -d 'Remove an installed tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "info" -d 'Show information about a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "auth" -d 'Configure authentication for a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and not __fish_seen_subcommand_from install list remove info auth help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s n -l name -d 'Tool name (defaults to directory/file name)' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l capabilities -d 'Path to capabilities JSON file (auto-detected if not specified)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s t -l target -d 'Target directory for installation (default: ~/.ironclaw/tools/)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l release -d 'Build in release mode (default: true)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l skip-build -d 'Skip compilation (use existing .wasm file)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s f -l force -d 'Force overwrite if tool already exists'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from install" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -s d -l dir -d 'Directory to list tools from (default: ~/.ironclaw/tools/)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -s v -l verbose -d 'Show detailed information'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -s d -l dir -d 'Directory to remove tool from (default: ~/.ironclaw/tools/)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from remove" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -s d -l dir -d 'Directory to look for tool (default: ~/.ironclaw/tools/)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from info" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -s d -l dir -d 'Directory to look for tool (default: ~/.ironclaw/tools/)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -s u -l user -d 'User ID for storing the secret (default: "default")' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from auth" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "install" -d 'Install a WASM tool from source directory or .wasm file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "list" -d 'List installed tools'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "remove" -d 'Remove an installed tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "info" -d 'Show information about a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "auth" -d 'Configure authentication for a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand tool; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "add" -d 'Add an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "remove" -d 'Remove an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "list" -d 'List configured MCP servers'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "auth" -d 'Authenticate with an MCP server (OAuth flow)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "test" -d 'Test connection to an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "toggle" -d 'Enable or disable an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and not __fish_seen_subcommand_from add remove list auth test toggle help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l client-id -d 'OAuth client ID (if authentication is required)' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l auth-url -d 'OAuth authorization URL (optional, can be discovered)' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l token-url -d 'OAuth token URL (optional, can be discovered)' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l scopes -d 'Scopes to request (comma-separated)' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l description -d 'Server description' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from add" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from remove" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -s v -l verbose -d 'Show detailed information'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -s u -l user -d 'User ID for storing the token (default: "default")' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from auth" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -s u -l user -d 'User ID for authentication (default: "default")' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from test" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -l enable -d 'Enable the server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -l disable -d 'Disable the server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from toggle" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "add" -d 'Add an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "remove" -d 'Remove an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "list" -d 'List configured MCP servers'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "auth" -d 'Authenticate with an MCP server (OAuth flow)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "test" -d 'Test connection to an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "toggle" -d 'Enable or disable an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand mcp; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "search" -d 'Search workspace memory (hybrid full-text + semantic)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "read" -d 'Read a file from the workspace'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "write" -d 'Write content to a workspace file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "tree" -d 'Show workspace directory tree'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "status" -d 'Show workspace status (document count, index health)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and not __fish_seen_subcommand_from search read write tree status help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -s l -l limit -d 'Maximum number of results' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from search" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from read" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -s a -l append -d 'Append instead of overwrite'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from write" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -s d -l depth -d 'Maximum depth to traverse' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from tree" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "search" -d 'Search workspace memory (hybrid full-text + semantic)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "read" -d 'Read a file from the workspace'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "write" -d 'Write content to a workspace file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "tree" -d 'Show workspace directory tree'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show workspace status (document count, index health)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand memory; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -f -a "list" -d 'List pending pairing requests'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -f -a "approve" -d 'Approve a pairing request by code'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and not __fish_seen_subcommand_from list approve help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -l json -d 'Output as JSON'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from approve" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from help" -f -a "list" -d 'List pending pairing requests'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from help" -f -a "approve" -d 'Approve a pairing request by code'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand pairing; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "install" -d 'Install the OS service (launchd on macOS, systemd on Linux)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "start" -d 'Start the installed service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "stop" -d 'Stop the running service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "status" -d 'Show service status'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "uninstall" -d 'Uninstall the OS service and remove the unit file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and not __fish_seen_subcommand_from install start stop status uninstall help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from install" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from uninstall" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "install" -d 'Install the OS service (launchd on macOS, systemd on Linux)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "start" -d 'Start the installed service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Stop the running service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show service status'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "uninstall" -d 'Uninstall the OS service and remove the unit file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand service; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand doctor" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand status" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -l shell -d 'The shell to generate completions for' -r -f -a "bash\t''
zsh\t''
fish\t''
powershell\t''
elvish\t''"
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand completion" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l job-id -d 'Job ID to execute' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l orchestrator-url -d 'URL of the orchestrator\'s internal API' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l max-iterations -d 'Maximum iterations before stopping' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand worker" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l job-id -d 'Job ID to execute' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l orchestrator-url -d 'URL of the orchestrator\'s internal API' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l max-turns -d 'Maximum agentic turns for Claude Code' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l model -d 'Claude model to use (e.g. "sonnet", "opus")' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -s m -l message -d 'Single message mode - send one message and exit' -r
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -s c -l config -d 'Configuration file path (optional, uses env vars by default)' -r -F
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l cli-only -d 'Run in interactive CLI mode only (disable other channels)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l no-db -d 'Skip database connection (for testing)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -l no-onboard -d 'Skip first-run onboarding check'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand claude-bridge" -s h -l help -d 'Print help'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "run" -d 'Run the agent (default if no subcommand given)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "onboard" -d 'Interactive onboarding wizard'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "config" -d 'Manage configuration settings'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "tool" -d 'Manage WASM tools'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "mcp" -d 'Manage MCP servers (hosted tool providers)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "memory" -d 'Query and manage workspace memory'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "pairing" -d 'DM pairing (approve inbound requests from unknown senders)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "service" -d 'Manage OS service (launchd / systemd)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "doctor" -d 'Probe external dependencies and validate configuration'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "status" -d 'Show system health and diagnostics'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "completion" -d 'Generate shell completion scripts'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "worker" -d 'Run as a sandboxed worker inside a Docker container (internal use). This is invoked automatically by the orchestrator, not by users directly'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "claude-bridge" -d 'Run as a Claude Code bridge inside a Docker container (internal use). Spawns the `claude` CLI and streams output back to the orchestrator'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and not __fish_seen_subcommand_from run onboard config tool mcp memory pairing service doctor status completion worker claude-bridge help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "init" -d 'Generate a default config.toml file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "list" -d 'List all settings and their current values'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "get" -d 'Get a specific setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "set" -d 'Set a setting value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "reset" -d 'Reset a setting to its default value'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "path" -d 'Show the settings storage info'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from tool" -f -a "install" -d 'Install a WASM tool from source directory or .wasm file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from tool" -f -a "list" -d 'List installed tools'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from tool" -f -a "remove" -d 'Remove an installed tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from tool" -f -a "info" -d 'Show information about a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from tool" -f -a "auth" -d 'Configure authentication for a tool'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "add" -d 'Add an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "remove" -d 'Remove an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "list" -d 'List configured MCP servers'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "auth" -d 'Authenticate with an MCP server (OAuth flow)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "test" -d 'Test connection to an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from mcp" -f -a "toggle" -d 'Enable or disable an MCP server'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "search" -d 'Search workspace memory (hybrid full-text + semantic)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "read" -d 'Read a file from the workspace'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "write" -d 'Write content to a workspace file'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "tree" -d 'Show workspace directory tree'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from memory" -f -a "status" -d 'Show workspace status (document count, index health)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from pairing" -f -a "list" -d 'List pending pairing requests'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from pairing" -f -a "approve" -d 'Approve a pairing request by code'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from service" -f -a "install" -d 'Install the OS service (launchd on macOS, systemd on Linux)'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from service" -f -a "start" -d 'Start the installed service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from service" -f -a "stop" -d 'Stop the running service'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from service" -f -a "status" -d 'Show service status'
complete -c ironclaw -n "__fish_ironclaw_using_subcommand help; and __fish_seen_subcommand_from service" -f -a "uninstall" -d 'Uninstall the OS service and remove the unit file'
