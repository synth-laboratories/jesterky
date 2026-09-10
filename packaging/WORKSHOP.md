# Optional Workshop distribution

Build each native artifact with `python3 scripts/package_workshop.py --output dist/workshop`.
The receipt pins version, platform, size and SHA-256. Development builds can also
use `--register-dev`; in Workshop select Jesterky's development channel and install.
No shell path or download URL is accepted from an MCP caller.

For the v0.10 release train, distribute macOS as explicitly non-notarized
(ad-hoc signing only); Developer ID signing is not configured. Generate the
final digest after packaging, upload native binaries to the GitHub release, then
copy their JSON entries into Workshop's
`apps/synth_desktop/src-tauri/resources/jesterky-release.json` artifacts array.
Do not populate that catalog until the URLs serve those exact bytes. A Workshop
build with an empty catalog deliberately reports release unavailability.

Local host installation does not mutate remote containers. Images enabling the
annotation runner must install their platform's pinned binary and configure
`SYNTH_ANNOTATION_JESTERKY=on`, `SYNTH_ANNOTATION_JESTERKY_COMMAND`, and the existing
annotation reservation proxy. The analysis default is Luna/low; credentials and
paid caps remain controlled by the host reservation flow.

## Version 0.1.3

This release adds the annotation inspection MCP bridge, namespaced tool and
reasoning-effort round trips through the explicit OpenRouter route, schema
instructions for every worker, durable pre-request cost/token reservations,
and successful worker outputs in the event journal for partial retry recovery.
Native analysis defaults remain Luna with low effort. OpenRouter is selected
explicitly as `openrouter/openai/gpt-5.6-luna`; it is not an implicit fallback.

`.github/workflows/workshop-release.yml` tests and packages macOS arm64 and
Linux amd64/arm64. A version tag builds verification artifacts. To publish all
three native targets, dispatch the workflow with an existing release tag after
authorization. Every platform builds that exact tagged source. The publisher
verifies all three receipts and preserves existing assets: identical bytes are
skipped and mismatched bytes fail before any upload. macOS is non-notarized;
users may instead build locally with the script above. Add only verified serving release URLs to Workshop's
catalog. The locally generated receipt names
the intended release URL; generating that receipt does not publish its bytes.

For remote Linux images, `scripts/install_container_runtime.py --help` describes
the receipt-based installer. It checks the native target, official release URL,
exact size and SHA-256 before atomic installation. A published release is
required; a host plugin installation alone does not provision remote workers.

Budgeted Containers runs require `jesterky 0.1.3`. The host supplies a pinned
price table and durable ledger via `JESTERKY_PROXY_BUDGET_JSON`. Reservations
are conservative maxima, never refunded after uncertain requests, and are not
reported as actual billing. The child worker receives no provider key. Its
read-only, loopback MCP inspection capability shares the annotation job's
existing tool limits and accounting.
