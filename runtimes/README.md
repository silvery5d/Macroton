# Runtime catalog

Each runtime release has a versioned manifest. A manifest records identity,
source, license, archive format, SHA-256 digest, and the relative Wine
entrypoint. The installer never derives trust from a filename.

```text
schema=1
id=wine-wow64-10.0
family=wine-wow64
version=10.0
source=https://publisher.example/runtime.tar.xz
license=LGPL-2.1-or-later
license_url=https://www.winehq.org/license
sha256=<64 lowercase hexadecimal digits>
archive=tar.xz
entrypoint=bin/wine
```

The committed example contains a placeholder checksum and cannot verify a real
artifact. Production manifests must be generated from a legally redistributable
artifact, reviewed, and committed separately.

Installation guarantees:

- checksum verification happens before extraction;
- only regular files and directories are accepted;
- absolute and parent-traversing archive paths are rejected;
- installation occurs in a private staging directory;
- final placement is an atomic rename;
- an existing nonmatching runtime is never overwritten.

## User-installed runtimes

Macroton can reference a Wine runtime already installed by the user without
copying or redistributing it:

```sh
macroton runtime discover
macroton runtime probe-win32 /absolute/path/to/wine
macroton runtime register-external wine-devel-10.8 /absolute/path/to/wine
```

Registration stores only metadata under Macroton's runtime root. It records a
SHA-256 fingerprint of the executable and refuses to launch if the external
binary later changes. Macroton neither copies nor redistributes the external
runtime; its use remains governed by the provider's license and the user's
existing entitlement.

If an external runtime contains component manifests under
`share/macroton/components/*.conf`, each `file.<lib/wine relative path>=<sha256>`
entry is also verified every time the runtime is resolved. This prevents a
composed graphics component from drifting while the unchanged Wine entrypoint
still appears valid.

`probe-win32` locates the runtime's own PE32 i386 `cmd.exe`, executes it in an
ephemeral prefix with a 60-second timeout, checks its Windows version output,
then shuts down Wine and removes the probe prefix. Passing this probe proves the
Win32 execution path works; it does not prove that a particular game works.
