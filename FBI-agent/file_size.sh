#! /bin/bash
find ./voice_recordings -name "*$1*" -exec du -c -h {} +
