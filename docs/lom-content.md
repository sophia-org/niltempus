<!-- Provenance: the Lom-specific parts of Sophia docs/lom-content-implementation.md (the Lom content-lifecycle diagnostic, Lom's conformance client, Lom's readback conversion and renderer deadline, and the Lom status paragraphs of "Production admission remains closed") at de776c68afdf9a133818f86917893c3362dc9fb7 (the pin) (Sophia rule 13). Sophia keeps the generic content records, admission, pools, reducers and device-hidden offline gates. -->
# Lom content

Lom is a native Sophia shell component (bar, and in later work popouts) that
renders its content with Vello and hands complete candidates to Sophia over
the shell content path. Sophia owns the generic content contract; this page is
Lom's side and the external gates that exercise it.

## 9P transport

Lom `0d1ff046edd41de02fdf0948cdb0ff74d35a97e7` requires a nonempty
`SOPHIA_SHELL_9P_SOCKET` and refuses any presence of `SOPHIA_SHELL_SOCKET`,
including an empty value. It emits `lom_shell_transport schema=1
wire=9p2000.L revision=6 epoch=N` before GPU admission. The serve profile keeps
capabilities `0x783`; the CPU `content-proof` profile requests `0x81`.

Sophia at `740c52551b4a667ffefc2b388eb58e5e4611ca1c` supports the selected
9P wire in the content conformance host, `shell-gpu-content-proof` and the
single-shell Session path. The panel runner passes
`--shell-transport=9p2000.L`; the GPU proof passes `--transport=9p2000.L`.
Independent Lom and Bemenu components explicitly select the same wire in
profiles. Provlita is outside this conversion and retains its existing wire.

The verifier requires one 9P transport record before GPU admission and render
records, with its epoch matching Sophia's grant. Mutations cover a missing,
repeated, late, wrong-wire or mismatched-epoch record. Sophia's protected
content host has exchanged a complete candidate and resource release with
Lom's CPU proof. This does not establish GPU execution or live presentation.

The Lom runners now consume verified prepared inputs and prebuilt integration
tools. The bounded helper builds Sophia and its profile-composition probe from
the signed pin into private targets; the runners verify the prepared manifest
before execution and after the proof. They perform no direct cargo builds.
A freshly bound Lom artifact and any attended GPU run remain separate release
evidence.

## Lom's side of the content path

Lom converts Vello's straight-alpha RGBA readback into premultiplied B,G,R,A in
place, using rounded integer arithmetic in sRGB channel space and transparent
black at alpha zero. Its iterator produces maximal whole-row chunks without
duplicating the image. Content dimensions are checked against 4 MiB before GPU
rendering; the diagnostic preview has its own separately named bound.

The renderer uses the texture path and its own map/copy operation instead of
`imaging_vello`'s indefinite readback wait. One two-second deadline covers the
poll and callback. A failed job is retained and the renderer refuses new work.
This recovery budget is not presentation pacing, proof of GPU completion, a
bound on arbitrary driver calls or an aggregate GPU allocation budget.

Lom's conformance client derives the output identity from
`ContentOutputFacts`, raises a frame demand and uses only the returned permit;
it requests its panel allocation rather than having a fixture injected behind
it. Lom's `content-lifecycle` diagnostic, together with an independent C
client, runs in Sophia's deterministic lifecycle host (cross-language
acceptance, not native display acceptance).

## Evidence limits

No render-node grant, installed-profile change or shell replacement follows
from content acceptance alone. GPU permission is a separate explicit launch
grant, independent of content negotiation; the accepted policy permits direct
rendering on supported stock Linux without promising a hard aggregate GPU
memory limit. Native acceptance still requires retirement, activation,
anchoring, consumed outside dismissal, restart and output changes, observed on
the physical gates above once they are unblocked.
