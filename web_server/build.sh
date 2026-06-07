#! /bin/bash

cargo build --release && systemctl --user restart web_server.service
