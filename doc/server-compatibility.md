# Server compatibility work

Session list and get scenarios run the real binary against a local HTTP server. They cover unknown enum values and failure reasons, changed field shapes, extra fields, unreadable bodies, unexpected statuses and unfamiliar refusal codes. Printable JSON continues with one notice; an unreadable response exits with code 3.

Explicit generated Unknown variants remain pending [the protocol generator change](https://github.com/tumpel/protocol/pull/104), its release, and a bump to the exact release candidate tag. The current scenarios exercise the existing JSON display fallback; they do not prove handling of successfully decoded Unknown variants.

Stage 2 adds scenario coverage for turn, transcript, persona, secret set, subscription, trigger, host, harness, config, and events/wait. It also audits command-specific decoders and handles the new Unknown variants after the protocol pin changes.
