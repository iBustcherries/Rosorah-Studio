# Rosorah Studio 0.31.0-preview.3

Layers → + Add layer → Alerts adds or selects the project's shared Alerts box. Repeating it does not create another source or duplicate its layer in the same scene. Another scene receives an instance of the same source and can position/resize it independently. Choosing Alerts also directs existing Display alert automation actions to this shared box. Existing older duplicate source objects/layers are not deleted automatically.

Right-click a layer, or use its three-dot menu:
- Rename opens a centered name dialog. Empty names are rejected; Save/Enter applies, Cancel/Escape discards. Renaming changes the source name across scenes and updates its media audio-channel label if that label still matched the old source name.
- Delete removes the layer from the current scene. Shared source data, automations and instances in other scenes remain. Unlock a locked layer before deleting it.

Legacy text-alert actions now render through the shared Alerts box instead of also drawing a separate canvas preview banner. If no box exists, the automation history directs you to Layers → + Add layer → Alerts.

Includes the preview sizing, audio monitoring and shared alert-box improvements from preview 2. Windows native-backend source remains included; this executable bundle is Linux-only.
