# Riot game trust anchor

`riotgames.pem` is Riot's published root, downloaded from
<https://static.developer.riotgames.com/docs/lol/riotgames.pem> on 2026-10-17.
PEM line endings were normalized to LF; certificate bytes are unchanged.

The PEM owns the certificate metadata. Inspect its DER SHA-256 fingerprint,
validity, subject and issuer from the repository root rather than maintaining a
prose copy:

```sh
openssl x509 -in crates/lucas-game/certs/riotgames.pem -noout \
  -sha256 -fingerprint -dates -subject -issuer
```

This is the **only** production trust anchor. The client uses rustls, bypasses
proxies, refuses redirects, and connects only to `https://127.0.0.1:2999`.
The hostname exception is required for Riot's game certificate at a loopback IP;
chain/signature/expiry checks remain enabled. An expired leaf fails normally.
The application can surface that error and explicitly select window-only mode;
this crate never installs a trust-all verifier or silently enables the game gate.

Changing the root requires comparing that generated metadata with Riot's published
certificate and updating the provenance above in review. There is no runtime
certificate download.
