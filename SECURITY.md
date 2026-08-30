# Security

## Reporting

Please report anything security-relevant privately, through GitHub's
[private vulnerability reporting](https://github.com/bybrooklyn/terrustia/security/advisories/new),
rather than as a public issue. I will acknowledge within a few days.

## What is in scope

This is a game server that strangers connect to over the open internet. **[`AUDIT.md`](AUDIT.md)**
lists what's already been found and fixed, so you can see the actual track record rather than take
this document's word for it. The things worth reporting here are the ones not already in it — the
ones that let somebody on the other end of a socket do something they should not:

- Crashing or hanging the server, or making it use unbounded memory or CPU
- Corrupting or destroying a world file
- Reading or writing data belonging to another player or another server
- Escalating to permissions they were not granted, or claiming a server they do not own
- Anything in the release or update path — a way to get an unsigned or substituted binary trusted

## What is deliberately not in scope

Some of these look like vulnerabilities and are not. They are properties of Terraria's own
multiplayer design, and diverging from them would change how the game plays:

- **The client is authoritative for its own position, inventory, and the damage it deals.** Vanilla
  trusts all of it. This server does too, on purpose — see `README.md`. A cheating client is a
  moderation problem, not a vulnerability. (Server-authoritative validation is planned as an
  opt-in, much later.)
- **The protocol has no encryption.** There is none to add without breaking every client. See "The
  plaintext transport" below for what that means in practice and what this server does about it.
- **Whoever can read the server's console can control the server.** That is the design: they can
  already read the world file.

Where this server is *more* strict than vanilla, that strictness is a feature and regressions in
it are in scope — for example, chest writes require the chest to be open, and tile-edit spam is
rate-limited the way `RemoteClient` does it.

## The plaintext transport

Terraria's own wire protocol (release 326/325, the one every unmodified client speaks) has no
encryption anywhere in it, and this server cannot add any without breaking every client that
connects to it (a modified client is not something this project is in the business of shipping).
That is a real, permanent limitation, not an oversight, and it is worth being plain about what it
actually means rather than leaving it to be discovered:

- **A password crosses the network in the clear.** `/login`'s password, `/register`'s new
  password, and a configured server join password all travel as ordinary chat-shaped text inside
  otherwise-ordinary game packets: nothing about them is distinguishable from other traffic to
  the protocol itself, but none of it is encrypted either. Anyone who can observe the raw TCP
  stream between a client and this server (another process on a shared machine, a router or ISP
  on the path, anyone on the same unencrypted Wi-Fi) can read it as it goes by.
- **An active attacker on the path can do more than watch.** Without transport encryption there is
  no cryptographic guarantee that the client is even talking to the real server rather than
  something relaying and altering the conversation in between (a machine-in-the-middle). This is
  materially more concerning on a network you do not trust (public Wi-Fi, a compromised router)
  than on your own LAN or over a VPN you already trust.
- **Reusing a password that matters elsewhere is the actual risk.** The protocol-level exposure
  above is a property of Terraria, unrelated to this implementation; the practical consequence is
  the same regardless of which server software is on the other end. Use a password made for this
  game and nowhere else, the same advice that would apply to a vanilla dedicated server.

What this server does about the part of that risk it *can* actually reduce:

- **Passwords are never stored, or transmitted again, in the clear.** Every account password is
  hashed with Argon2 (`admin::Account::new`/`verify_hash`) the moment it is received; the plaintext
  never touches disk, and the stored PHC hash is not reversible.
- **Passwords are never logged.** No `tracing::` call or audit-log entry on the login, register or
  claim paths ever carries a password or the claim token, at any level, including inside an error
  branch: see `admin::mod`'s own "never logged" convention for where this is enforced and why.
- **Login attempts are throttled, not merely trusted to fail slowly.** Every place a password or
  the claim token is checked (`/login`, the join password, the panel's own login) backs off
  exponentially per address *and* per account name, with jitter, in memory, resetting the moment a
  correct credential arrives; the claim token compare itself is constant-time rather than a leaky
  `!=`. There is no lockout: an attacker spamming a known account name cannot lock its real owner
  out of it, only slow both of them down together, and a restart clears the backoff state
  harmlessly. See `admin::throttle`'s own doc comment for the full mechanism.
- **The web admin panel never faces the network at all.** It binds loopback only
  (`127.0.0.1`/`::1`), refused twice over (once by config validation, once again by the panel's
  own bind call), so there is no remote panel surface to intercept in the first place. Reaching it
  from another machine means tunnelling (SSH `-L`, a VPN, a reverse proxy you terminate TLS on
  yourself), which is the operator's own trusted channel, not something this project has to secure
  on their behalf.

**Planned, not yet built:** a one-time join credential, a short-lived, single-use token issued
through the panel and typed into Terraria's normal password prompt in place of a reusable password,
so an observed credential is worthless the moment it is used once. This does not encrypt the
connection and does not stop an active machine-in-the-middle from seeing everything else in the
session; it only takes a reusable secret off the wire. Tracked in `TODO.md` under "deferred with
reasons written down".

## Supported versions

Only the most recent release. This project is early; there is no maintenance branch yet.

## Verifying a release

Releases are signed with cosign keyless, so there is no key to trust — the signature proves the
artefact was built by this repository's own release workflow:

```sh
cosign verify-blob \
  --bundle terrustia-<version>-<target>.tar.gz.cosign.bundle \
  --certificate-identity-regexp 'https://github.com/bybrooklyn/terrustia/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  terrustia-<version>-<target>.tar.gz
```
