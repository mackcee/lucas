# Riot game trust anchor

`riotgames.pem` is Riot's published root, downloaded from
<https://static.developer.riotgames.com/docs/lol/riotgames.pem> on 2026-10-17.
PEM line endings were normalized to LF; certificate bytes are unchanged.

Certificate SHA-256 fingerprint (DER):
`CA:8C:9D:32:5B:4C:DC:46:4C:6C:94:A5:85:C8:5E:91:EC:23:D4:0B:A5:BF:3A:E2:82:2B:95:1A:4A:50:4E:A3`.
Validity: 2013-12-04 through 2043-11-27. Subject/issuer: LoL Game Engineering
Certificate Authority, Riot Games.

This is the **only** production trust anchor. The client uses rustls, bypasses
proxies, refuses redirects, and connects only to `https://127.0.0.1:2999`.
The hostname exception is required for Riot's game certificate at a loopback IP;
chain/signature/expiry checks remain enabled. An expired leaf fails normally.
The application can surface that error and explicitly select window-only mode;
this crate never installs a trust-all verifier or silently enables the game gate.

Changing the root requires checking Riot's published certificate and updating
this file and fingerprint in review. There is no runtime certificate download.
