# TTFX live-theme color roles

The plugin must never blanket-tint an effect. `A` colors may follow the Omarchy
palette; `P` colors carry the effect's physical/semantic identity and stay
unchanged; `N` colors are structural neutrals/backgrounds and stay unchanged.
Unknown or pre-existing input colors always pass through.

| Effect | Theme (`A`) | Preserve (`P` / `N`) |
|---|---|---|
| beams | beam + final gradients | black fill |
| binarypath | final text | digital-green binary characters; white collapse flash |
| blackhole | final text | border, starfield, explosion/collapse palette, black fades |
| bouncyballs | ball + final gradients | input colors only |
| bubbles | ordinary bubble + final colors | explicit rainbow mode; white pop flash |
| burn | starting/final/smoke accents, per product decision | complete white/yellow/orange/red fire and ember spectrum; black |
| colorshift | final text when distinguishable | active rainbow (core identity); default final palette collides, preserve wins |
| crumble | final text and derived dust accents | gray fallback, white reset flash, black |
| decrypt | discovered/final text | green ciphertext; white discovery flash |
| errorcorrect | final text | red error, green correction and white error flash |
| expand | all authored colors | dynamic input colors |
| fireworks | final text | shell/explosion colors and white flare |
| highlight | base/final/highlight brightness ramp | dynamic input colors |
| laseretch | final text | laser, sparks, hot/cooling colors and white |
| middleout | starting + final gradients | dynamic input colors |
| orbittingvolley | launcher + final gradient | dynamic input colors |
| overflow | moving overflow + final gradients | black sentinel; dynamic input colors |
| pour | starting + final gradients | dynamic input colors |
| print | typing head + final gradient | dynamic input colors |
| randomsequence | final/discovery gradient | terminal background; dynamic input colors |
| rings | ring + final gradients | dynamic input colors |
| scattered | all authored gradients | dynamic input colors |
| slice | final gradient | dynamic input colors |
| slide | all authored gradients | dynamic input colors |
| smoke | starting + final/paint accents | smoke-density grayscale and black; dynamic input colors |
| spotlights | final gradient + brightness variants | dynamic input colors |
| spray | final/droplet gradients | dynamic input colors |
| swarm | final text | blue swarm, yellow flash, white clear ramp |
| sweep | final text | grayscale shimmer and black fill |
| synthgrid | text/final gradient | neon grid gradient; white collision preserves grid |
| thunderstorm | final/storm text | lightning, rain, sparks, orange strike glow, gray/black storm neutrals |
| unstable | final text | orange instability/explosion and gray fallback |
| vhstape | final text | RGB glitch sequence, grayscale noise, white redraw and gray fallback |
| waves | final text only when distinguishable | yellow/blue wave palette; default collision preserves wave |
| wipe | all authored gradients | dynamic input colors |

## Implementation contract

1. Color transforms run at frame emission, not scene construction: TTFX caches
   `CharacterVisual.formatted_symbol`, so build-time SGR hooks cannot update live.
2. Per-effect mapping is preserve-first: `semantic set -> identity`, then
   `accent map -> theme palette`, then unknown RGB -> identity.
3. Generated gradients use TTFX's integer-floor `Gradient` implementation; never
   approximate them with float interpolation.
4. RGB collisions are resolved conservatively (preserve wins). Exact handling of
   mixed-role transitions requires provenance/role metadata at emission time.
5. `matrix` and `rain` remain the plugin's hand-rolled audio-reactive versions;
   all other 35 upstream effects are exposed in `Panel.qml`.
