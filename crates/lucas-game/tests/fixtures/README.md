# Live Client fixtures

`gamestats.json`, `activeplayername.json`, `playerlist.json`, and `eventdata.json`
are the corresponding subsets of Riot's public recorded example response:
<https://static.developer.riotgames.com/docs/lol/liveclientdata_sample.json>,
retrieved 2026-10-17. Formatting was normalized; response field values were not
changed. `activeplayername` is extracted from `activePlayer.summonerName`.
No webcam or private match capture is included.

`events-scripted.json` is **synthetic**, using Riot's documented event shapes to
exercise kills, objectives, streamer names, unknown events, and game end.
Tests modify copies of the public fixture to cover modern Riot IDs, pauses,
respawns, different maps/modes, missing identity, and reconnects.

The HTTPS mock in `src/api_tests.rs` uses generated test certificates and serves
these subset endpoints over a real loopback TCP/TLS connection. Production has
no alternate endpoint or root configuration surface.
