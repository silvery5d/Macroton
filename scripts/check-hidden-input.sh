#!/bin/zsh
set -eu

# Local-only diagnostic. Python's getpass reads the terminal without echoing;
# the entered text is never printed, persisted, or sent anywhere. Only byte
# lengths, non-printable-byte counts, and one-way digests are retained.
exec python3 - <<'PY'
import getpass
import hashlib

def describe(value: str):
    data = value.encode("utf-8")
    controls = sum(byte < 32 or byte == 127 for byte in data)
    whitespace = sum(chr(byte).isspace() for byte in data)
    return len(data), controls, whitespace, hashlib.sha256(data).hexdigest()

print("Enter the same value twice. Input is hidden and never stored.")
first = getpass.getpass("Value 1: ")
second = getpass.getpass("Value 2: ")
first_bytes, first_controls, first_whitespace, first_hash = describe(first)
second_bytes, second_controls, second_whitespace, second_hash = describe(second)
print(f"first_bytes={first_bytes}")
print(f"second_bytes={second_bytes}")
print(f"first_controls={first_controls}")
print(f"second_controls={second_controls}")
print(f"first_whitespace={first_whitespace}")
print(f"second_whitespace={second_whitespace}")
print(f"same_length={'yes' if first_bytes == second_bytes else 'no'}")
print(f"same_digest={'yes' if first_hash == second_hash else 'no'}")
print(f"digest={first_hash}")
PY
