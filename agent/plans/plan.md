## a few notes

See the un-staged git diffs for this work tree.
/home/pete/me/worktrees/beet/no-std-update/beet
That work tree is for the version of beet that you have permission to make changes to if needed.



- blinky.rs still has too much boilerplate. EVERYTHING must be behing an Esp32Plugin. the plugin may have config fields if required.
- LedColor should be a component not a resource
- Ws2812 should eventually also be a component, but leave the async issue for now, ill deal with that seperately
- instead of a HueFadePlugin, use a HueFade component
- all startup stuff should be in startup systems (as much as possible, except stuff that must happen before constructing bevy apps)