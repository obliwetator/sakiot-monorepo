#! /bin/bash

cargo build --release && systemctl --user restart fbi-agent.service
