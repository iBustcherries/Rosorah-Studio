# Rosorah Studio 0.15.1 validation

## Camera compatibility changes

- HTTP camera input uses format detection instead of always forcing multipart demuxing. Detected multipart streams are still configured as a single live stream, avoiding startup waits for end-of-stream.
- Known V4L2 loopback output devices remain selectable before a producer attaches. Metadata-only nodes and unrelated output devices remain excluded.
- Errors name the input type and failing component and provide USB or Wi-Fi guidance.
- USB through the DroidCam Linux client still requires a loaded v4l2loopback module and an active virtual-camera producer. This release does not install or load kernel drivers.

## Checks

- Locked release build; rustfmt check and all-target Clippy with warnings denied.
- 66 ordinary tests passed: 11 core, 4 capture, 51 app. Eight existing environment-dependent tests remain ignored.
- Actual local HTTP multipart MJPEG and raw JPEG streams passed through GStreamer, RGBA conversion, crop/flip and the capture engine. Tests checked pixel content and dimensions, the shared recording reader, clearing on disconnect and bounded shutdown. Local requests ran without the environment's HTTP proxy.
- Existing camera negotiation, configuration, URL and crop validation tests passed. Capability tests cover capture-ready devices, output-only loopback devices, unrelated outputs and metadata nodes.
- Native renderer libraries, launch observer, installer and launcher are byte-identical to 0.15.0. Game capture, audio, recording, streaming and account implementation code are unchanged.
- ZIP/tar archives are checked for full decompression, member hashes and permissions. Saved downloads are compared with the original archives before delivery.

## Limits and upgrade

No physical DroidCam phone or USB camera is available here. Passing local stream tests does not establish that the reported phone-specific decoder error is resolved. A missing virtual camera kernel module remains a separate machine setup issue.

Close Rosorah, extract the complete archive into a fresh folder and run `run-studio.sh`. From 0.13.0 or later, Steam, games and KDE can stay open. Existing project settings are retained. See `validation-0.15.0.md` for the original camera/account feature scope and external verification limits.
