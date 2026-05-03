#! /bin/bash

cargo build --features dev-login && systemctl --user restart web_server-debug.service
