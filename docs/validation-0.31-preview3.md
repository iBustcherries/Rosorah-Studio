# Validation — 0.31.0-preview.3

Linux release build completed successfully.

Native UI check under Xvfb/Mesa, using an isolated project and real mouse/keyboard input:
- Open Layers → + Add layer; the direct Alerts item is visible.
- Add Alerts twice; saved project has exactly one Alerts source and one layer in the scene.
- Add Alerts to Canvas 2; both scenes refer to the same source ID.
- Open a layer's right-click menu; Rename and Delete are visible.
- Rename to “shared alerts”, save the dialog and project; source name persisted.
- Delete its layer from Canvas 1 and save; its source and Canvas 2 layer remain intact.
- Inspected the menu and rename dialog in the running app.

The prior preview-2 automated suite passed 189 tests. That full suite was not repeated for this small UI update; the release compile and targeted native workflow above verify the changed paths. Windows backend runtime testing remains pending; this is a Linux executable bundle.
