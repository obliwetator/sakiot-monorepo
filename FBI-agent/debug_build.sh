#! /bin/bash

cargo build && systemctl --user restart fbi-agent-debug.service
