# KeePassXC test fixtures

Files in this directory are copied verbatim from the KeePassXC repository,
`tests/data/` on the `develop` branch, fetched 2026-09-14. They are used as an
interoperability corpus: chiave must open what KeePassXC opens and KeePassXC
must open what chiave writes. KeePassXC is GPL-2.0-or-later / GPL-3.0; these
files are test data only and are not linked into the chiave binary.
`manifest.txt` lists the source URL of every file.

Credentials, taken from KeePassXC's own tests:

| File | Password | Key file | Format |
|---|---|---|---|
| Format200.kdbx | a | | KDBX 2 (unsupported by keepass-rs) |
| Format300.kdbx | a | | KDBX 3.1 |
| Format400.kdbx | t | | KDBX 4 |
| NewDatabase.kdbx | a | | KDBX 3.1 |
| Compressed.kdbx | (empty) | | KDBX 3.1 |
| ProtectedStrings.kdbx | masterpw | | KDBX 3.1 |
| NonAscii.kdbx | Δöض | | KDBX 3.1 |
| BrokenHeaderHash.kdbx | (empty) | | KDBX 3.1, header hash deliberately wrong |
| FileKeyXml.kdbx | | FileKeyXml.key | KDBX 2 |
| FileKeyBinary.kdbx | | FileKeyBinary.key | KDBX 2 |
| FileKeyHex.kdbx | | FileKeyHex.key | KDBX 2 |
| FileKeyHashed.kdbx | | FileKeyHashed.key | KDBX 2 |
| FileKeyXmlV2.kdbx | | FileKeyXmlV2.keyx | KDBX 4 |
| YubiKeyProtectedPasswords.kdbx | | YubiKey challenge-response | KDBX 4 |

Unlisted files: see the corresponding `Test*.cpp` in KeePassXC for credentials.
