# sakiot-speakers

Bot accounts that talk in a voice channel, to measure what recording costs the
real bot: Discord's voice servers, UDP, decryption, decoding, the recorder and
its writes. The k6 suite in `load_testing` covers the web server and the disk
with a simulated recorder; this covers the bot.

Each token is its own Discord application. The run steps through how many bots
talk at once (default 0, 1, 2, 4, 8, five minutes each), each looping the same
audio. An Ogg Opus file at 48 kHz is passed through packet for packet, so the
recorder receives what a Discord client sent.

## Once

1. Create the bot applications in the Discord developer portal (New
   Application, then Bot, then Reset Token). They need no privileged intents.
2. Invite each to the staging guild with View Channel, Connect and Speak:
   `https://discord.com/oauth2/authorize?client_id=<application id>&scope=bot&permissions=3146752`
3. Put the tokens in a file outside the repository, one per line, mode 0600.
4. The staging bot must be a `loadtest` build, the only kind that records bots.
   The deploy engine builds staging that way once it has been refreshed with
   `sudo ops/update-deploy-engine.sh`.

The audio should be speech without long pauses, from a real Discord
recording, so the packets match a client's. Cut a minute out of one with a
stream copy, which keeps the packets as they were:

```sh
ffmpeg -ss 600 -t 60 -i <recording>.ogg -c copy speech.ogg
```

## A run

The bot leaves a channel with no people in it, so join the staging voice
channel yourself first and stay muted. Then:

```sh
cargo run --release -p sakiot-speakers -- \
  --tokens ~/.config/sakiot/speaker-tokens \
  --guild <staging guild id> --channel <voice channel id> \
  | tee run.log
ops/sakiot-speakers/measure.sh run.log
```

`measure.sh` reads the window of each step from `run.log` and asks the VPS's
Prometheus (over `SAKIOT_DEV_SSH`) what the staging bot spent in it: CPU and
tokio busy time in cores, packets and RTP kbit/s received, disk writes, resident
memory, recordings, and the p99 time of each 20 ms voice tick's receive and
record stages. Differences between steps are the cost of one more speaker.

For where the CPU goes, profile the staging bot on the VPS during the top step:

```sh
pid="$(systemctl show -p MainPID --value 'sakiot-staging-fbi-agent@*' | grep -v '^0$' | head -n 1)"
sudo perf record -F 199 -g -p "$pid" -- sleep 60
sudo perf report --stdio --no-children | head -n 60
```

The bots' recordings are ordinary staging recordings; delete them from the
dashboard afterwards.
