


So this crate is currently empty with a few basic examples. The goal is for it to sit on top of the beet repository, and to use all beet and Bevy conventions for all the applications. Let's start very simple. I want you to include a local beet worktree as a dependency: `/home/pete/me/worktrees/beet/no-std-update/beet`, and update the blinky example to use the bevy color instead of these custom color types.

Now this crate is no_std. There's a lot of untested ground here, you will likely need to make changes to beet feature flags etc to get it working. That means making changes in the sibling repository, which you have permission to do (only in the worktree i provided you)

We should be able to use bevy color utilities instead of these hand rolled ones.

```rust
use beet::prelude::*;


let my_color = Color::srgb_u8(255,255,255);
```


## Verification

Ensure the same `blinky` example still compiles and runs, and you can get output.


You also have access to the Bevy repository which has Bevy Color and you can use that for reference if you need: `../bevy/crates/bevy_color`

```
// bevy_color module docs

Crate bevy_color Copy item path
Source
Search
Settings
Help

Summary
Representations of colors in various color spaces.

This crate provides a number of color representations, including:

Srgba (standard RGBA, with gamma correction)
LinearRgba (linear RGBA, without gamma correction)
Hsla (hue, saturation, lightness, alpha)
Hsva (hue, saturation, value, alpha)
Hwba (hue, whiteness, blackness, alpha)
Laba (lightness, a-axis, b-axis, alpha)
Lcha (lightness, chroma, hue, alpha)
Oklaba (lightness, a-axis, b-axis, alpha)
Oklcha (lightness, chroma, hue, alpha)
Xyza (x-axis, y-axis, z-axis, alpha)
Each of these color spaces is represented as a distinct Rust type.

Color Space Usage
Rendering engines typically use linear RGBA colors, which allow for physically accurate lighting calculations. However, linear RGBA colors are not perceptually uniform, because both human eyes and computer monitors have non-linear responses to light. “Standard” RGBA represents an industry-wide compromise designed to encode colors in a way that looks good to humans in as few bits as possible, but it is not suitable for lighting calculations.

Most image file formats and scene graph formats use standard RGBA, because graphic design tools are intended to be used by humans. However, 3D lighting calculations operate in linear RGBA, so it is important to convert standard colors to linear before sending them to the GPU. Most Bevy APIs will handle this conversion automatically, but if you are writing a custom shader, you will need to do this conversion yourself.

HSL and LCH are “cylindrical” color spaces, which means they represent colors as a combination of hue, saturation, and lightness (or chroma). These color spaces are useful for working with colors in an artistic way - for example, when creating gradients or color palettes. A gradient in HSL space from red to violet will produce a rainbow. The LCH color space is more perceptually accurate than HSL, but is less intuitive to work with.

HSV and HWB are very closely related to HSL in their derivation, having identical definitions for hue. Where HSL uses saturation and lightness, HSV uses a slightly modified definition of saturation, and an analog of lightness in the form of value. In contrast, HWB instead uses whiteness and blackness parameters, which can be used to lighten and darken a particular hue respectively.

Oklab and Oklch are perceptually uniform color spaces that are designed to be used for tasks such as image processing. They are not as widely used as the other color spaces, but are useful for tasks such as color correction and image analysis, where it is important to be able to do things like change color saturation without causing hue shifts.

XYZ is a foundational space commonly used in the definition of other more modern color spaces. The space is more formally known as CIE 1931, where the x and z axes represent a form of chromaticity, while y defines an illuminance level.

See also the Wikipedia article on color spaces.

Conversion
Conversion between the various color spaces is achieved using Rust’s native From trait. Because certain color spaces are defined by their transformation to and from another space, these From implementations reflect that set of definitions.

let color = Srgba::rgb(0.5, 0.5, 0.5);

// Using From explicitly
let linear_color = LinearRgba::from(color);

// Using Into
let linear_color: LinearRgba = color.into();
For example, the sRGB space is defined by its relationship with Linear RGB, and HWB by its with sRGB. As such, it is the responsibility of sRGB to provide From implementations for Linear RGB, and HWB for sRGB. To then provide conversion between Linear RGB and HWB directly, HWB is responsible for implementing these conversions, delegating to sRGB as an intermediatory. This ensures that all conversions take the shortest path between any two spaces, and limit the proliferation of domain specific knowledge for each color space to their respective definitions.

Conversion
Conversion
Conversion
Conversion
Conversion
Conversion
Conversion
Conversion
Conversion
Linear
sRGB
Oklab
Oklch
XYZ
Lab
Lch
sRGB
HWB
HSV
HSL
GPU
Other Utilities
The crate also provides a number of color operations, such as blending, color difference, and color range operations.

In addition, there is a Color enum that can represent any of the color types in this crate. This is useful when you need to store a color in a data structure that can’t be generic over the color type.

Color types that are either physically or perceptually linear also implement Add<Self>, Sub<Self>, Mul<f32> and Div<f32> allowing you to use them with splines.

Please note that most often adding or subtracting colors is not what you may want. Please have a look at other operations like blending, lightening or mixing colors using e.g. Mix or Luminance instead.

Example
use bevy_color::{Srgba, Hsla};

let srgba = Srgba::new(0.5, 0.2, 0.8, 1.0);
let hsla: Hsla = srgba.into();

println!("Srgba: {:?}", srgba);
println!("Hsla: {:?}", hsla);

```
