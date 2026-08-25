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
- **The protocol has no encryption.** There is none to add without breaking every client.
  `/login` sends a password as ordinary chat text, so anyone able to observe the TCP stream can
  read it — do not reuse a password that matters, and tunnel the connection if that concerns you.
  This is stated in the README rather than left to be discovered.
- **Whoever can read the server's console can control the server.** That is the design: they can
  already read the world file.

Where this server is *more* strict than vanilla, that strictness is a feature and regressions in
it are in scope — for example, chest writes require the chest to be open, and tile-edit spam is
rate-limited the way `RemoteClient` does it.

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
