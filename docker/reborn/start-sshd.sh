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

if [ ! -d "$IRONCLAW_REBORN_HOME" ]; then
  echo "IRONCLAW_REBORN_HOME must exist before SSH starts: $IRONCLAW_REBORN_HOME" >&2
  exit 1
fi

if [ -e "$ssh_dir" ] || [ -L "$ssh_dir" ]; then
  if [ ! -d "$ssh_dir" ] || [ -L "$ssh_dir" ]; then
    echo "refusing unsafe SSH state path: $ssh_dir" >&2
    exit 1
  fi
  ssh_dir_owner="$(stat -c '%u' "$ssh_dir")"
  ssh_dir_mode="$(stat -c '%a' "$ssh_dir")"
  if [ "$ssh_dir_owner" != "0" ]; then
    echo "refusing SSH state directory not owned by root: $ssh_dir" >&2
    exit 1
  fi
  case "$ssh_dir_mode" in
    *[2367][0-7]|*[0-7][2367])
      echo "refusing group/world-writable SSH state directory: $ssh_dir" >&2
      exit 1
      ;;
  esac
else
  mkdir -m 755 "$ssh_dir"
fi
chmod 755 "$ssh_dir"
mkdir -p /run/sshd
chmod 755 /run/sshd

host_key_tmp="$ssh_dir/ssh_host_ed25519_key.tmp.$$"
authorized_keys_tmp="$ssh_dir/authorized_keys.tmp.$$"
config_tmp="$ssh_dir/sshd_config.tmp.$$"
trap 'rm -f "$host_key_tmp" "$host_key_tmp.pub" "$authorized_keys_tmp" "$config_tmp"' EXIT HUP INT TERM

if [ -e "$host_key" ] || [ -L "$host_key" ]; then
  if [ ! -f "$host_key" ] || [ -L "$host_key" ]; then
    echo "refusing unsafe SSH host key path: $host_key" >&2
    exit 1
  fi
  if ! ssh-keygen -l -f "$host_key" >/dev/null 2>&1; then
    echo "persisted SSH host key is invalid: $host_key" >&2
    exit 1
  fi
else
  ssh-keygen -q -t ed25519 -N '' -f "$host_key_tmp"
  chmod 600 "$host_key_tmp"
  mv "$host_key_tmp" "$host_key"
  rm -f "$host_key_tmp.pub"
fi
chmod 600 "$host_key"

# ssh-keygen -l -f (below) exits 0 for a private key file too, so a pasted
# private key must be rejected here, before anything derived from it is ever
# written to disk. Never echo $public_key itself into a diagnostic.
case "$public_key" in
  *'-----BEGIN'*)
    echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY looks like a private key (PEM header found); supply the matching .pub public key instead" >&2
    exit 1
    ;;
esac

# Operators routinely paste the key straight out of `cat id_ed25519.pub`, which
# carries a trailing newline, and some consoles add surrounding blank lines.
# Those are valid single-line keys, so strip surrounding whitespace BEFORE the
# single-line check below -- otherwise the most common paste is rejected, and
# because the entrypoint runs this under `set -e` a rejection aborts the whole
# container boot, not just SSH.
key_lines="$(printf '%s' "$public_key" | awk 'NF { gsub(/^[ \t\r]+|[ \t\r]+$/, ""); print }')"
if [ -z "$key_lines" ]; then
  echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY contains no key material" >&2
  exit 1
fi
if [ "$(printf '%s\n' "$key_lines" | wc -l)" -gt 1 ]; then
  echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY must contain exactly one OpenSSH public key" >&2
  exit 1
fi
public_key="$key_lines"

if [ "$(printf '%s\n' "$public_key" | wc -l)" -gt 1 ]; then
  echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY must be a single-line OpenSSH public key (<type> <base64> [comment]); it looks like a multi-line private key" >&2
  exit 1
fi
case "$(printf '%s\n' "$public_key" | awk '{print NF}')" in
  0|1)
    echo "IRONCLAW_REBORN_SSH_PUBLIC_KEY must be an OpenSSH public key in '<type> <base64> [comment]' form" >&2
    exit 1
    ;;
esac

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
    'AllowTcpForwarding no' \
    'AllowAgentForwarding no' \
    'PermitTunnel no' \
    'PrintMotd no' \
    "SetEnv IRONCLAW_REBORN_HOME=$IRONCLAW_REBORN_HOME CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup" \
    'Subsystem sftp internal-sftp'
} > "$config_tmp"
chmod 600 "$config_tmp"
mv "$config_tmp" "$config"

/usr/sbin/sshd -t -f "$config"
/usr/sbin/sshd -f "$config"

trap - EXIT HUP INT TERM
