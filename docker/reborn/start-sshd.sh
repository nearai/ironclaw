#!/bin/sh
set -eu

public_key="${IRONCLAW_REBORN_SSH_PUBLIC_KEY:-}"
if [ -z "$public_key" ]; then
  exit 0
fi

ssh_dir="$IRONCLAW_REBORN_HOME/ssh"
host_key="$ssh_dir/ssh_host_ed25519_key"
authorized_keys="$ssh_dir/authorized_keys"
config="$ssh_dir/sshd_config"

mkdir -p "$ssh_dir" /run/sshd
chmod 755 "$ssh_dir"

if [ ! -s "$host_key" ]; then
  ssh-keygen -q -t ed25519 -N '' -f "$host_key"
fi
chmod 600 "$host_key"

authorized_keys_tmp="$authorized_keys.tmp.$$"
config_tmp="$config.tmp.$$"
trap 'rm -f "$authorized_keys_tmp" "$config_tmp"' EXIT HUP INT TERM

printf '%s\n' "$public_key" > "$authorized_keys_tmp"
if ! ssh-keygen -l -f "$authorized_keys_tmp" >/dev/null 2>&1; then
  echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY is not a valid OpenSSH public key" >&2
  exit 1
fi
chmod 644 "$authorized_keys_tmp"
mv "$authorized_keys_tmp" "$authorized_keys"

{
  printf '%s\n' \
    'Port 2222' \
    'ListenAddress 0.0.0.0' \
    "HostKey \"$host_key\"" \
    "PidFile \"$ssh_dir/sshd.pid\"" \
    "AuthorizedKeysFile \"$authorized_keys\"" \
    'AllowUsers agent' \
    'AuthenticationMethods publickey' \
    'PubkeyAuthentication yes' \
    'PasswordAuthentication no' \
    'KbdInteractiveAuthentication no' \
    'PermitEmptyPasswords no' \
    'PermitRootLogin no' \
    'StrictModes yes' \
    'UsePAM no' \
    'X11Forwarding no' \
    'PrintMotd no' \
    "SetEnv IRONCLAW_REBORN_HOME=$IRONCLAW_REBORN_HOME CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup" \
    'Subsystem sftp internal-sftp'
} > "$config_tmp"
chmod 600 "$config_tmp"
mv "$config_tmp" "$config"

/usr/sbin/sshd -t -f "$config"
/usr/sbin/sshd -f "$config"

trap - EXIT HUP INT TERM
