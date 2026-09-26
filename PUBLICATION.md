# Publication scope

This repository contains Macroton launcher source, configuration format/examples, build and diagnostic scripts, public eligibility metadata, and test fixtures.

It does not include runtime binaries, Steam/client/game payloads, local runtime manifests, local and unreviewed experimental compatibility profiles, or raw validation evidence. No compatibility claim follows from the presence of a script or example.

The root MIT license applies to Macroton-authored material. Third-party source excerpts and patch files require separate provenance and license review before publication.

Building the launcher requires a MinGW-w64 x86_64 compiler because Cargo builds the Macroton-authored Steam CEF helper from source and embeds that generated output. Set `MINGW64_CC` if the compiler is not `x86_64-w64-mingw32-gcc`.

Experimental patch-based build scripts are withheld with their upstream-derived patch sets pending license review. The baseline Wine build scripts remain included.
