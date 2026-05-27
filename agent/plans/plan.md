# Plan

## Context

This is a downstream library from our primary project called beet. We're working off a work tree so that we can make changes freely. you have permission to make changes to the worktree as required, but do not not commit changes so i can review them.


`/home/pete/me/worktrees/beet/embedded/beet`
## Next steps

So we're following on from the first go at converting pure ESP32 examples into Bevy.It's not a bad start, but we can do a lot better. Consider that the LED thing really should be component based, not resource based.

- blinky.rs still has too much boilerplate. as much as possible must be behing an Esp32Plugin and ideally ran in a Startup system. the plugin may have config fields if required.
- LedColor should be a component not a resource
- Ws2812 should eventually also be a component, but leave the async issue for now, ill deal with that seperately
- instead of a HueFadePlugin, use a HueFade component
- all startup stuff should be in startup systems (as much as possible, except stuff that must happen before constructing bevy apps)


## Verification

Ensure everything is compiling, re-upload to the device which is plugged in, and verify all good
