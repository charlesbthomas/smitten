//! Turns a game's WGSL effect into a fragment shader SDL can load.
//!
//! The game writes one function. This module wraps it in an entry point
//! with the texture, sampler and uniform bindings SDL's GPU API expects,
//! checks it, and translates it with naga to the language the GPU driver
//! takes: SPIR-V for Vulkan, or the Metal Shading Language for Metal.
//!
//! A game may also declare textures of its own, such as
//! `var sky: texture_cube<f32>;` or `var fog: texture_3d<f32>;`, with no binding. Each gets a sampler
//! named after it, `sky_sampler`, and the next free texture slot. A
//! `texture_depth_2d` gets a `sampler_comparison`.

use naga::back::{msl, spv};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use std::ffi::CString;
use std::mem::offset_of;

use smitten_core::backend::{
    ShaderTexture, TextureKind, UniformArray, UniformField, UniformKind, VERTEX_COLOR,
    VERTEX_POSITION, VERTEX_TEX_COORD, Vertex, VertexBufferLayout, VertexDataType, VertexLayout,
    VertexLayoutAttribute, VertexStep,
};

/// The name of the fragment entry point in the WGSL the framework writes.
const ENTRY_POINT: &str = "smitten_fragment";

/// The shading language a shader is translated to, which depends on the
/// GPU driver SDL picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderLanguage {
    /// SPIR-V, for Vulkan.
    Spirv,
    /// Metal Shading Language source, for Metal. SDL compiles it when the
    /// shader is created.
    Msl,
}

/// A shader translated for the GPU driver, ready to hand to SDL.
#[derive(Debug)]
pub(crate) struct ShaderCode {
    pub language: ShaderLanguage,
    /// SPIR-V words in little-endian order, or MSL source text.
    pub bytes: Vec<u8>,
    /// The name of the entry point, which the translation may have
    /// changed to avoid a word the language reserves.
    pub entry_point: CString,
}

/// The version of the Metal Shading Language generated shaders are
/// written in. 2.1 is the oldest every Mac SDL's Metal driver runs on
/// supports.
const MSL_VERSION: (u8, u8) = (2, 1);

/// Appended after the game's source, so its line numbers stay the same in
/// error messages. WGSL does not care about declaration order. `IMAGE` is
/// replaced with the type of the texture being drawn, and `GROUP` with
/// the group SDL wants textures in: 2 for fragment shaders and 0 for
/// vertex shaders.
const WRAPPER: &str = "
@group(GROUP) @binding(0) var image: IMAGE;
@group(GROUP) @binding(1) var image_sampler: sampler;
";

/// What a fragment shader is given, with the game's `Varyings` after the
/// built-in values.
fn fragment_input(varyings: &[Varying]) -> String {
    format!(
        "
struct SmittenFragmentInput {{
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
{}}}
",
        varying_fields(varyings)
    )
}

/// The members carrying `varyings` between the stages, from location 2.
fn varying_fields(varyings: &[Varying]) -> String {
    varyings
        .iter()
        .enumerate()
        .map(|(index, varying)| {
            let flat = if varying.integer {
                " @interpolate(flat)"
            } else {
                ""
            };
            format!(
                "    @location({}){flat} smitten_varying{index}: {},\n",
                index + 2,
                varying.ty
            )
        })
        .collect()
}

/// Copies what the vertex shader wrote into the fragment shader's
/// `varyings`, before the game's `effect` reads them.
fn varying_copies(varyings: &[Varying]) -> String {
    varyings
        .iter()
        .enumerate()
        .map(|(index, varying)| {
            format!(
                "    varyings.{} = input.smitten_varying{index};\n",
                varying.name
            )
        })
        .collect()
}

/// The built-in fragment shader, as a game would write it, for drawing
/// to several canvases at once. Drawing to one uses the GLSL one beside
/// this file.
pub(crate) const BUILTIN: &str = "
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSample(image, image_sampler, uv) * color;
}
";

/// As [`BUILTIN`], for drawing a layer of an array texture: the layer is
/// the one uniform, as in `sprite_array.frag`.
pub(crate) const BUILTIN_ARRAY: &str = "
struct Params { layer: f32 }

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSample(image, image_sampler, uv, i32(params.layer)) * color;
}
";

/// The entry point for a shader with one output, `effect`, drawing to
/// `targets` canvases: each gets the same color.
fn single_entry(targets: u32, prologue: &str) -> String {
    let fields: String = (0..targets)
        .map(|index| format!("    @location({index}) out{index}: vec4<f32>,\n"))
        .collect();
    let copies: String = (0..targets)
        .map(|index| format!("    output.out{index} = result;\n"))
        .collect();
    format!(
        "
struct SmittenFragmentOutput {{
{fields}}}

@fragment
fn smitten_fragment(input: SmittenFragmentInput) -> SmittenFragmentOutput {{
{prologue}    let result = effect(input.color, input.uv, input.position.xy);
    var output: SmittenFragmentOutput;
{copies}    return output;
}}
"
    )
}

/// The entry point for a shader with several outputs, `effects`, whose
/// `Outputs` struct has the members `outputs`, drawing to `targets`
/// canvases. A canvas past the last member gets transparent black.
fn multiple_entry(outputs: &[String], targets: u32, prologue: &str) -> String {
    let fields: String = (0..targets)
        .map(|index| format!("    @location({index}) out{index}: vec4<f32>,\n"))
        .collect();
    let copies: String = (0..targets as usize)
        .map(|index| match outputs.get(index) {
            Some(name) => format!("    output.out{index} = result.{name};\n"),
            None => format!("    output.out{index} = vec4<f32>(0.0);\n"),
        })
        .collect();
    format!(
        "
struct SmittenFragmentOutput {{
{fields}}}

@fragment
fn smitten_fragment(input: SmittenFragmentInput) -> SmittenFragmentOutput {{
{prologue}    let result: Outputs = effects(input.color, input.uv, input.position.xy);
    var output: SmittenFragmentOutput;
{copies}    return output;
}}
"
    )
}

/// The name of the vertex entry point in the WGSL the framework writes.
const VERTEX_ENTRY_POINT: &str = "smitten_vertex";

/// The built-in vertex shader for ordinary draws, whose vertices are
/// already placed in pixels, as `sprite.vert` beside this file does it.
/// Vulkan uses the SPIR-V compiled from that; other languages translate
/// this.
const SPRITE_VERTEX: &str = "
struct SmittenScreen {
    // Two divided by the drawable size in pixels.
    pixel_to_clip: vec2<f32>,
}

@group(1) @binding(0) var<uniform> smitten_screen: SmittenScreen;

struct SmittenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn smitten_vertex(
    @location(0) in_position: vec2<f32>,
    @location(1) in_uv: vec2<f32>,
    @location(2) in_color: vec4<f32>,
) -> SmittenVertexOutput {
    // Pixels run from the top-left with y down; clip space has y up.
    // Depth 0.5 is where z 0 lands in the projection, as for meshes.
    let clip = in_position * smitten_screen.pixel_to_clip - 1.0;
    var output: SmittenVertexOutput;
    output.position = vec4<f32>(clip.x, -clip.y, 0.5, 1.0);
    output.uv = in_uv;
    output.color = in_color;
    return output;
}
";

// Compiled from the GLSL beside them by `shaders/build.sh`.
static VERTEX_SPIRV: &[u8] = include_bytes!("../shaders/sprite.vert.spv");
static FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite.frag.spv");
static ARRAY_FRAGMENT_SPIRV: &[u8] = include_bytes!("../shaders/sprite_array.frag.spv");

/// The shaders for ordinary draws: the vertex shader, the fragment shader,
/// and the fragment shader for drawing a layer of an array texture.
pub(crate) fn sprite_shaders(language: ShaderLanguage) -> Result<[ShaderCode; 3], String> {
    match language {
        ShaderLanguage::Spirv => {
            let code = |bytes: &[u8]| ShaderCode {
                language,
                bytes: bytes.to_vec(),
                entry_point: c"main".to_owned(),
            };
            Ok([
                code(VERTEX_SPIRV),
                code(FRAGMENT_SPIRV),
                code(ARRAY_FRAGMENT_SPIRV),
            ])
        }
        ShaderLanguage::Msl => {
            let module = naga::front::wgsl::parse_str(SPRITE_VERTEX)
                .map_err(|error| error.emit_to_string(SPRITE_VERTEX))?;
            let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
                .validate(&module)
                .map_err(|error| error.emit_to_string(SPRITE_VERTEX))?;
            let vertex = translate(&module, &info, naga::ShaderStage::Vertex, 0, language)?;
            let fragment = compile_for(BUILTIN, 1, false, language)?.code;
            let array = compile_for(BUILTIN_ARRAY, 1, true, language)?.code;
            Ok([vertex, fragment, array])
        }
    }
}

/// Translates a checked module's entry point for `stage` to `language`.
/// `textures` is how many of the game's own textures it declares, which
/// sit after `image`.
///
/// SDL's GPU API wants resources in fixed places, which differ by
/// language. The WGSL the framework writes uses SPIR-V's: textures in
/// group 0 for vertex shaders and 2 for fragment shaders, each texture's
/// sampler at the binding after it, and uniforms in groups 1 and 3.
fn translate(
    module: &naga::Module,
    info: &naga::valid::ModuleInfo,
    stage: naga::ShaderStage,
    textures: usize,
    language: ShaderLanguage,
) -> Result<ShaderCode, String> {
    let (entry, texture_group, uniform_group) = match stage {
        naga::ShaderStage::Vertex => (VERTEX_ENTRY_POINT, 0, 1),
        _ => (ENTRY_POINT, 2, 3),
    };
    let failed = |error: &dyn std::fmt::Display| format!("could not translate the shader: {error}");
    match language {
        ShaderLanguage::Spirv => {
            let mut options = spv::Options::default();
            combine_samplers(&mut options, texture_group, textures);
            if stage == naga::ShaderStage::Vertex {
                // SDL's clip space already has y up, as WGSL's does, so naga
                // must not flip it the way it would for plain Vulkan.
                options
                    .flags
                    .remove(spv::WriterFlags::ADJUST_COORDINATE_SPACE);
            }
            let pipeline = spv::PipelineOptions {
                shader_stage: stage,
                entry_point: entry.to_string(),
            };
            let words = spv::write_vec(module, info, &options, Some(&pipeline))
                .map_err(|error| failed(&error))?;
            Ok(ShaderCode {
                language,
                bytes: words.iter().flat_map(|word| word.to_le_bytes()).collect(),
                entry_point: CString::new(entry).expect("entry point names have no NUL"),
            })
        }
        ShaderLanguage::Msl => {
            let mut resources = msl::EntryPointResources::default();
            metal_slots(&mut resources, texture_group, uniform_group, textures);
            let options = msl::Options {
                lang_version: MSL_VERSION,
                per_entry_point_map: [(entry.to_string(), resources)].into(),
                // A resource without a slot is a bug in this module; fail
                // rather than write a shader Metal would reject.
                fake_missing_bindings: false,
                ..msl::Options::default()
            };
            let pipeline = msl::PipelineOptions {
                entry_point: Some((stage, entry.to_string())),
                ..msl::PipelineOptions::default()
            };
            let (source, translated) = msl::write_string(module, info, &options, &pipeline)
                .map_err(|error| failed(&error))?;
            let index = module
                .entry_points
                .iter()
                .position(|point| point.name == entry)
                .ok_or_else(|| format!("the shader has no `{entry}`"))?;
            let name = match translated.entry_point_names.get(index) {
                Some(Ok(name)) => name.clone(),
                Some(Err(error)) => return Err(failed(error)),
                None => return Err(format!("the shader has no `{entry}`")),
            };
            Ok(ShaderCode {
                language,
                bytes: source.into_bytes(),
                entry_point: CString::new(name).map_err(|error| failed(&error))?,
            })
        }
    }
}

/// The Metal slots SDL binds a stage's resources to. Textures and their
/// samplers are numbered together from 0, `image` first, and uniform
/// buffers from 0 in binding order. SDL puts vertex buffers from
/// `[[buffer(14)]]`, which naga's `[[stage_in]]` inputs leave to it.
fn metal_slots(
    resources: &mut msl::EntryPointResources,
    texture_group: u32,
    uniform_group: u32,
    textures: usize,
) {
    for slot in 0..=textures as u8 {
        let (texture, sampler) = if slot == 0 {
            (0, 1)
        } else {
            (u32::from(slot) * 2, u32::from(slot) * 2 + 1)
        };
        resources.resources.insert(
            naga::ResourceBinding {
                group: texture_group,
                binding: texture,
            },
            msl::BindTarget {
                texture: Some(slot),
                ..msl::BindTarget::default()
            },
        );
        resources.resources.insert(
            naga::ResourceBinding {
                group: texture_group,
                binding: sampler,
            },
            msl::BindTarget {
                sampler: Some(msl::BindSamplerTarget::Resource(slot)),
                ..msl::BindTarget::default()
            },
        );
    }
    // A vertex shader has the framework's uniforms and then the game's
    // `params`; a fragment shader has only `params`.
    for binding in 0..2 {
        resources.resources.insert(
            naga::ResourceBinding {
                group: uniform_group,
                binding,
            },
            msl::BindTarget {
                buffer: Some(binding as u8),
                ..msl::BindTarget::default()
            },
        );
    }
}

/// The part of every mesh vertex shader that does not depend on the
/// layout. It places each vertex with the draw's matrix and tints its
/// color, where ordinary draws are placed and tinted on the CPU. Its
/// outputs match what every fragment shader takes. SDL wants vertex shader
/// uniforms in group 1.
const MESH_VERTEX_COMMON: &str = "
struct SmittenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}
";

/// What every vertex shader reads for each draw.
const DRAW_UNIFORMS: &str = "
struct SmittenDraw {
    transform_projection: mat4x4<f32>,
    tint: vec4<f32>,
}

@group(1) @binding(0) var<uniform> smitten_draw: SmittenDraw;
";

/// A vertex shader made for one vertex layout.
#[derive(Debug)]
pub(crate) struct MeshVertexShader {
    pub code: ShaderCode,
    /// The index into the layout's attributes of what each input location
    /// reads, in location order. Attributes the shader does not read are
    /// left out.
    pub locations: Vec<usize>,
}

/// How a vertex attribute reaches a shader: the WGSL type it is read as,
/// and how many values that type has.
pub(crate) fn attribute_shader_type(
    data_type: VertexDataType,
    components: u8,
) -> (&'static str, u8) {
    match (data_type, components) {
        (VertexDataType::Float, 1) => ("f32", 1),
        (VertexDataType::Float, 2) => ("vec2<f32>", 2),
        (VertexDataType::Float, 3) => ("vec3<f32>", 3),
        (VertexDataType::Float, _) => ("vec4<f32>", 4),
        (VertexDataType::UNorm8, _) => ("vec4<f32>", 4),
        (VertexDataType::UNorm16, 1 | 2) => ("vec2<f32>", 2),
        (VertexDataType::UNorm16, _) => ("vec4<f32>", 4),
    }
}

/// A WGSL `vecN<f32>` of `want` values: the attribute's own values where
/// it has them, read from `expr`, and `defaults` for the rest.
fn widen(expr: &str, read: u8, components: u8, defaults: &[f32]) -> String {
    let values: Vec<String> = defaults
        .iter()
        .enumerate()
        .map(|(index, default)| {
            if index < usize::from(components) {
                if read == 1 {
                    expr.to_string()
                } else {
                    format!("{expr}.{}", ["x", "y", "z", "w"][index])
                }
            } else {
                format!("{default:?}")
            }
        })
        .collect();
    match values.as_slice() {
        [value] => value.clone(),
        _ => format!("vec{}<f32>({})", defaults.len(), values.join(", ")),
    }
}

/// Turns a color given sRGB-encoded, as games give colors, into linear
/// light, for gamma-correct rendering. Alpha is left alone.
const LINEAR_COLOR: &str = "
fn smitten_linear(color: vec4<f32>) -> vec4<f32> {
    let low = color.rgb / 12.92;
    let high = pow((color.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(high, low, color.rgb <= vec3<f32>(0.04045)), color.a);
}
";

/// Writes the built-in mesh vertex shader for `layout`: it reads the
/// position, texture coordinate and color by their standard names, with
/// defaults for any the layout lacks. With `gamma_correct`, vertex colors
/// are turned into linear light.
pub(crate) fn mesh_vertex(
    layout: &VertexLayout,
    gamma_correct: bool,
    language: ShaderLanguage,
) -> Result<MeshVertexShader, String> {
    let mut source = String::from(MESH_VERTEX_COMMON);
    source.push_str(DRAW_UNIFORMS);
    source.push_str(LINEAR_COLOR);
    let mut inputs = Inputs::new(layout);
    let position = inputs.read(VERTEX_POSITION, &[0.0, 0.0, 0.0, 1.0]);
    let uv = inputs.read(VERTEX_TEX_COORD, &[0.0, 0.0]);
    let color = inputs.read(VERTEX_COLOR, &[1.0, 1.0, 1.0, 1.0]);
    let color = if gamma_correct {
        format!("smitten_linear({color})")
    } else {
        color
    };
    source.push_str(&format!(
        "
@vertex
fn smitten_vertex({}) -> SmittenVertexOutput {{
    var output: SmittenVertexOutput;
    output.position = smitten_draw.transform_projection * {position};
    output.uv = {uv};
    output.color = {color} * smitten_draw.tint;
    return output;
}}
",
        inputs.parameters.join(", ")
    ));
    let code = mesh_vertex_code(&source, language)?;
    Ok(MeshVertexShader {
        code,
        locations: inputs.locations,
    })
}

/// The inputs of a vertex shader being written for a layout: one location
/// for each of the layout's attributes it reads, however often.
struct Inputs<'a> {
    layout: &'a VertexLayout,
    parameters: Vec<String>,
    /// The index into the layout's attributes of each location.
    locations: Vec<usize>,
}

impl<'a> Inputs<'a> {
    fn new(layout: &'a VertexLayout) -> Inputs<'a> {
        Inputs {
            layout,
            parameters: Vec::new(),
            locations: Vec::new(),
        }
    }

    /// WGSL for the attribute called `name`, with as many values as
    /// `defaults` has: the attribute's own where it has them and
    /// `defaults` for the rest, or all of `defaults` if the layout lacks
    /// it.
    fn read(&mut self, name: &str, defaults: &[f32]) -> String {
        let found = self
            .layout
            .attributes
            .iter()
            .enumerate()
            .find(|(_, attribute)| attribute.name == name);
        let Some((index, attribute)) = found else {
            return widen("", 0, 0, defaults);
        };
        let (ty, width) = attribute_shader_type(attribute.data_type, attribute.components);
        let location = match self.locations.iter().position(|&known| known == index) {
            Some(location) => location,
            None => {
                let location = self.locations.len();
                self.parameters
                    .push(format!("@location({location}) in_{location}: {ty}"));
                self.locations.push(index);
                location
            }
        };
        widen(
            &format!("in_{location}"),
            width,
            attribute.components,
            defaults,
        )
    }
}

/// Translates a vertex shader the framework wrote to `language`.
fn mesh_vertex_code(source: &str, language: ShaderLanguage) -> Result<ShaderCode, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| error.emit_to_string_with_path(source, "mesh vertex shader"))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| error.emit_to_string_with_path(source, "mesh vertex shader"))?;
    translate(&module, &info, naga::ShaderStage::Vertex, 0, language)
        .map_err(|error| error.replace("the shader", "the mesh vertex shader"))
}

/// Added only when the game declares `struct Params`. SDL wants fragment
/// shader uniforms in group 3.
const PARAMS_BINDING: &str = "
@group(3) @binding(0) var<uniform> params: Params;
";

/// As [`PARAMS_BINDING`], for a game's vertex shader: SDL wants vertex
/// shader uniforms in group 1, after the framework's own.
const VERTEX_PARAMS_BINDING: &str = "
@group(1) @binding(1) var<uniform> params: Params;
";

/// The most `Varyings` members a shader may have. Vulkan and Metal both
/// promise 16 locations between the stages, and the built-in values take
/// two.
const MAX_VARYINGS: usize = 14;

/// A fragment shader ready to hand to SDL.
#[derive(Debug)]
pub(crate) struct Compiled {
    pub code: ShaderCode,
    /// The size of the `Params` struct in bytes, or 0 without one.
    pub uniform_size: u32,
    pub fields: Vec<UniformField>,
    /// The game's own textures, in the order of their slots after `image`.
    pub textures: Vec<ShaderTexture>,
    /// Whether the game wrote a vertex shader, `position`, to go with it.
    pub has_vertex: bool,
}

/// What a game's source declares that changes what is wrapped around it,
/// found by name before it is parsed.
struct Declared {
    params: bool,
    /// `effects` in place of `effect`.
    multiple: bool,
    effect: bool,
    position: bool,
    attributes: bool,
    varyings: bool,
    /// Whether it reads `instance_index`.
    instance_index: bool,
    /// The game's own textures, and whether each is a depth texture.
    textures: Vec<(String, bool)>,
}

impl Declared {
    fn of(source: &str) -> Declared {
        Declared {
            params: declares_struct(source, "Params"),
            multiple: defines_function(source, "effects"),
            effect: defines_function(source, "effect"),
            position: defines_function(source, "position"),
            attributes: declares_struct(source, "Attributes"),
            varyings: declares_struct(source, "Varyings"),
            instance_index: mentions(source, "instance_index"),
            textures: declared_textures(source),
        }
    }

    /// What both stages add after the game's source before anything
    /// else: `image`, the samplers of its own textures, `params`, and the
    /// `attributes` and `varyings` it reads and writes. A shader with
    /// only `position` gets the built-in `effect`.
    fn declarations(&self, group: u32, image: &str, params: &str) -> String {
        let mut text = WRAPPER
            .replace("GROUP", &group.to_string())
            .replace("IMAGE", image);
        for (name, depth) in &self.textures {
            // A depth texture is sampled by comparing.
            let sampler = if *depth {
                "sampler_comparison"
            } else {
                "sampler"
            };
            text.push_str(&format!("var {name}_sampler: {sampler};\n"));
        }
        if self.params {
            text.push_str(params);
        }
        if self.attributes {
            text.push_str("var<private> attributes: Attributes;\n");
        }
        if self.varyings {
            text.push_str("var<private> varyings: Varyings;\n");
        }
        if self.instance_index {
            text.push_str("var<private> instance_index: u32;\n");
        }
        if self.position && !self.effect && !self.multiple {
            text.push_str(BUILTIN);
        }
        text
    }
}

/// One member of a game's `struct Varyings`.
#[derive(Debug)]
struct Varying {
    name: String,
    /// Its WGSL type.
    ty: String,
    /// Whether it holds integers, which cannot be blended between
    /// vertices.
    integer: bool,
}

/// Parses the game's source with what the framework appended, `full`.
/// An error in the appended part means the game's functions do not fit
/// it, which `wrong` explains; the framework's code is not the game's to
/// read.
fn parse(full: &str, user_lines: u32, wrong: fn(&str) -> String) -> Result<naga::Module, String> {
    naga::front::wgsl::parse_str(full).map_err(|error| match error.location(full) {
        Some(location) if location.line_number > user_lines => wrong(error.message()),
        _ => error.emit_to_string_with_path(full, "shader"),
    })
}

/// Compiles a game's shader to draw to one canvas.
pub(crate) fn compile(source: &str, language: ShaderLanguage) -> Result<Compiled, String> {
    compile_for(source, 1, false, language)
}

/// Checks a game's shader the way drawing with it would, both stages.
pub(crate) fn check(source: &str, language: ShaderLanguage) -> Result<(), String> {
    let compiled = compile(source, language)?;
    if compiled.has_vertex {
        compile_vertex(source, &standard_layout(), false, language)?;
    }
    Ok(())
}

/// The layout of the vertices of ordinary draws, already placed in
/// pixels, as `sprite.vert` reads them.
pub(crate) fn standard_layout() -> VertexLayout {
    let attribute = |name: &str, offset, components| VertexLayoutAttribute {
        name: name.to_string(),
        buffer: 0,
        offset,
        data_type: VertexDataType::Float,
        components,
    };
    VertexLayout {
        buffers: vec![VertexBufferLayout {
            stride: size_of::<Vertex>() as u32,
            step: VertexStep::PerVertex,
        }],
        attributes: vec![
            attribute(VERTEX_POSITION, offset_of!(Vertex, position) as u32, 2),
            attribute(VERTEX_TEX_COORD, offset_of!(Vertex, uv) as u32, 2),
            attribute(VERTEX_COLOR, offset_of!(Vertex, color) as u32, 4),
        ],
    }
}

/// Compiles a game's shader, or one of the built-in ones, to draw to
/// `targets` canvases at once. With `layered`, the texture drawn is an
/// array texture.
pub(crate) fn compile_for(
    source: &str,
    targets: u32,
    layered: bool,
    language: ShaderLanguage,
) -> Result<Compiled, String> {
    let declared = Declared::of(source);
    if declared.varyings && !declared.position {
        return Err(wrong_position(
            "`struct Varyings` is written by the vertex shader, so it needs one",
        ));
    }
    let image = if layered {
        "texture_2d_array<f32>"
    } else {
        "texture_2d<f32>"
    };
    let mut full = String::from(source);
    full.push_str(&declared.declarations(2, image, PARAMS_BINDING));
    let user_lines = source.lines().count() as u32;
    let wrong = if declared.multiple {
        wrong_effects
    } else {
        wrong_effect
    };

    // The entry point copies the members of `Outputs` and `Varyings`, so
    // their names come from parsing the source without it first.
    if declared.multiple || declared.varyings {
        let module = parse(&full, user_lines, wrong)?;
        let outputs = if declared.multiple {
            Some(output_names(&module)?)
        } else {
            None
        };
        let varyings = if declared.varyings {
            varyings(&module)?
        } else {
            Vec::new()
        };
        full.push_str(&fragment_input(&varyings));
        let prologue = varying_copies(&varyings);
        full.push_str(&match outputs {
            Some(outputs) => multiple_entry(&outputs, targets, &prologue),
            None => single_entry(targets, &prologue),
        });
    } else {
        full.push_str(&fragment_input(&[]));
        full.push_str(&single_entry(targets, ""));
    }

    let mut module = parse(&full, user_lines, wrong)?;
    let textures = bind_textures(&mut module, &declared.textures, 2)?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| {
            let report = error.emit_to_string_with_path(&full, "shader");
            if report.contains("smitten_fragment") {
                wrong("its arguments or return type do not match")
            } else {
                report
            }
        })?;

    let (uniform_size, fields) = if declared.params {
        params_layout(&module)?
    } else {
        (0, Vec::new())
    };

    let code = translate(
        &module,
        &info,
        naga::ShaderStage::Fragment,
        textures.len(),
        language,
    )?;

    Ok(Compiled {
        code,
        uniform_size,
        fields,
        textures,
        has_vertex: declared.position,
    })
}

/// WGSL has a texture and a sampler as two things; SDL binds them as
/// one. Vulkan lets both variables name the same combined binding, so
/// `image` and the game's `textures` own textures in `group` each map
/// their pair onto one slot.
fn combine_samplers(options: &mut spv::Options, group: u32, textures: usize) {
    options.binding_map.insert(
        naga::ResourceBinding { group, binding: 1 },
        spv::BindingInfo {
            descriptor_set: group,
            binding: 0,
            binding_array_size: None,
        },
    );
    for slot in 1..=textures as u32 {
        for binding in [slot * 2, slot * 2 + 1] {
            options.binding_map.insert(
                naga::ResourceBinding { group, binding },
                spv::BindingInfo {
                    descriptor_set: group,
                    binding: slot,
                    binding_array_size: None,
                },
            );
        }
    }
}

/// Compiles the vertex shader of a game's shader, its `position`, to
/// read vertices laid out as `layout`. The built-in values are read as
/// [`mesh_vertex`] reads them, and with `linearize` the vertex color is
/// turned into linear light. The game's `Attributes` are filled from the
/// layout's attributes of the same names.
pub(crate) fn compile_vertex(
    source: &str,
    layout: &VertexLayout,
    linearize: bool,
    language: ShaderLanguage,
) -> Result<MeshVertexShader, String> {
    let declared = Declared::of(source);
    let mut full = String::from(source);
    full.push_str(&declared.declarations(0, "texture_2d<f32>", VERTEX_PARAMS_BINDING));
    full.push_str(DRAW_UNIFORMS);
    full.push_str(LINEAR_COLOR);
    let user_lines = source.lines().count() as u32;
    let module = parse(&full, user_lines, wrong_position)?;
    let varyings = if declared.varyings {
        varyings(&module)?
    } else {
        Vec::new()
    };
    let attributes = if declared.attributes {
        attribute_members(&module)?
    } else {
        Vec::new()
    };

    let mut inputs = Inputs::new(layout);
    let mut body = String::new();
    for (name, components) in &attributes {
        let defaults = &default_value(name)[..usize::from(*components)];
        body.push_str(&format!(
            "    attributes.{name} = {};\n",
            inputs.read(name, defaults)
        ));
    }
    let position = inputs.read(VERTEX_POSITION, &[0.0, 0.0, 0.0, 1.0]);
    let uv = inputs.read(VERTEX_TEX_COORD, &[0.0, 0.0]);
    let mut color = inputs.read(VERTEX_COLOR, &[1.0, 1.0, 1.0, 1.0]);
    if linearize {
        color = format!("smitten_linear({color})");
    }
    if declared.instance_index {
        inputs
            .parameters
            .push("@builtin(instance_index) smitten_instance: u32".to_string());
        body.push_str("    instance_index = smitten_instance;\n");
    }
    let copies: String = varyings
        .iter()
        .enumerate()
        .map(|(index, varying)| {
            format!(
                "    output.smitten_varying{index} = varyings.{};\n",
                varying.name
            )
        })
        .collect();
    full.push_str(&format!(
        "
struct SmittenVertexOutput {{
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
{fields}}}

@vertex
fn smitten_vertex({parameters}) -> SmittenVertexOutput {{
{body}    var output: SmittenVertexOutput;
    output.uv = {uv};
    output.color = {color} * smitten_draw.tint;
    output.position = position(smitten_draw.transform_projection, {position});
{copies}    return output;
}}
",
        fields = varying_fields(&varyings),
        parameters = inputs.parameters.join(", "),
    ));

    let mut module = parse(&full, user_lines, wrong_position)?;
    bind_textures(&mut module, &declared.textures, 0)?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| {
            let report = error.emit_to_string_with_path(&full, "shader");
            if report.contains("smitten_vertex") {
                wrong_position("its arguments or return type do not match")
            } else {
                report
            }
        })?;
    let code = translate(
        &module,
        &info,
        naga::ShaderStage::Vertex,
        declared.textures.len(),
        language,
    )?;
    Ok(MeshVertexShader {
        code,
        locations: inputs.locations,
    })
}

/// What an attribute a game's shader reads but the vertices lack reads as:
/// white for the color, and (0, 0, 0, 1) for anything else.
fn default_value(name: &str) -> [f32; 4] {
    if name == VERTEX_COLOR {
        [1.0; 4]
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// The members of the game's `struct Varyings`, which must be numbers or
/// vectors of them.
fn varyings(module: &naga::Module) -> Result<Vec<Varying>, String> {
    let members = struct_members(module, "Varyings")
        .ok_or_else(|| wrong_position("`Varyings` must be a struct"))?;
    if members.len() > MAX_VARYINGS {
        return Err(format!(
            "`Varyings` has {} members, but at most {MAX_VARYINGS} can be passed to `effect`",
            members.len()
        ));
    }
    members
        .into_iter()
        .map(|(name, ty)| {
            let (scalar, size) = match *ty {
                naga::TypeInner::Scalar(scalar) => (scalar, 1),
                naga::TypeInner::Vector { size, scalar } => (scalar, size as u8),
                _ => (naga::Scalar::BOOL, 0),
            };
            let kind = match (scalar.kind, scalar.width) {
                (naga::ScalarKind::Float, 4) => "f32",
                (naga::ScalarKind::Sint, 4) => "i32",
                (naga::ScalarKind::Uint, 4) => "u32",
                _ => {
                    return Err(format!(
                        "`Varyings.{name}` must be an `f32`, `i32` or `u32`, \
                         or a vector of two to four of them"
                    ));
                }
            };
            let ty = match size {
                1 => kind.to_string(),
                size => format!("vec{size}<{kind}>"),
            };
            Ok(Varying {
                name,
                ty,
                integer: kind != "f32",
            })
        })
        .collect()
}

/// The members of the game's `struct Attributes`, with how many values
/// each has. They must be `f32` or vectors of it, as vertex attributes
/// are read as floats.
fn attribute_members(module: &naga::Module) -> Result<Vec<(String, u8)>, String> {
    let members = struct_members(module, "Attributes")
        .ok_or_else(|| wrong_position("`Attributes` must be a struct"))?;
    members
        .into_iter()
        .map(|(name, ty)| match *ty {
            naga::TypeInner::Scalar(naga::Scalar::F32) => Ok((name, 1)),
            naga::TypeInner::Vector {
                size,
                scalar: naga::Scalar::F32,
            } => Ok((name, size as u8)),
            _ => Err(format!(
                "`Attributes.{name}` must be an `f32`, `vec2<f32>`, `vec3<f32>` or `vec4<f32>`"
            )),
        })
        .collect()
}

/// The names and types of the members of the struct called `name`.
fn struct_members<'a>(
    module: &'a naga::Module,
    name: &str,
) -> Option<Vec<(String, &'a naga::TypeInner)>> {
    let found = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(name))
        .map(|(_, ty)| &ty.inner);
    let Some(naga::TypeInner::Struct { members, .. }) = found else {
        return None;
    };
    Some(
        members
            .iter()
            .map(|member| {
                (
                    member.name.clone().unwrap_or_default(),
                    &module.types[member.ty].inner,
                )
            })
            .collect(),
    )
}

/// The names of the textures `source` declares at the top level with no
/// binding, in order, and whether each is a depth texture, found well
/// enough for the purpose: `var`, a name, then a texture type. A
/// declaration with a binding is left alone, and one the scan misses
/// fails to compile with a message about bindings.
fn declared_textures(source: &str) -> Vec<(String, bool)> {
    let words: Vec<&str> = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let mut names: Vec<(String, bool)> = Vec::new();
    for window in words.windows(3) {
        if window[0] == "var"
            && window[2].starts_with("texture_")
            && !names.iter().any(|(name, _)| name == window[1])
        {
            names.push((
                window[1].to_string(),
                window[2].starts_with("texture_depth_"),
            ));
        }
    }
    names
}

/// Gives each of the game's own textures in `names`, and its sampler,
/// the slot after the ones before it. Returns what kind each one is.
fn bind_textures(
    module: &mut naga::Module,
    names: &[(String, bool)],
    group: u32,
) -> Result<Vec<ShaderTexture>, String> {
    let mut textures = Vec::new();
    for (name, _) in names {
        let found = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some(name) && global.binding.is_none())
            .map(|(handle, global)| (handle, global.ty));
        let Some((handle, ty)) = found else {
            continue;
        };
        let (kind, comparison) = match module.types[ty].inner {
            naga::TypeInner::Image {
                dim: naga::ImageDimension::D2,
                arrayed: false,
                class: naga::ImageClass::Depth { multi: false },
            } => (TextureKind::Flat, true),
            naga::TypeInner::Image {
                dim,
                arrayed,
                class:
                    naga::ImageClass::Sampled {
                        kind: naga::ScalarKind::Float,
                        multi: false,
                    },
            } => match (dim, arrayed) {
                (naga::ImageDimension::D2, false) => (TextureKind::Flat, false),
                (naga::ImageDimension::D2, true) => (TextureKind::Array, false),
                (naga::ImageDimension::Cube, false) => (TextureKind::Cube, false),
                (naga::ImageDimension::D3, false) => (TextureKind::Volume, false),
                _ => return Err(unsupported_texture(name)),
            },
            _ => return Err(unsupported_texture(name)),
        };
        let slot = textures.len() as u32 + 1;
        module.global_variables.get_mut(handle).binding = Some(naga::ResourceBinding {
            group,
            binding: slot * 2,
        });
        let sampler_name = format!("{name}_sampler");
        let sampler = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some(sampler_name.as_str()))
            .map(|(handle, _)| handle);
        if let Some(sampler) = sampler {
            module.global_variables.get_mut(sampler).binding = Some(naga::ResourceBinding {
                group,
                binding: slot * 2 + 1,
            });
        }
        textures.push(ShaderTexture {
            name: name.clone(),
            kind,
            comparison,
        });
    }
    Ok(textures)
}

fn unsupported_texture(name: &str) -> String {
    format!(
        "texture `{name}` must be a `texture_2d<f32>`, `texture_2d_array<f32>`, \
         `texture_cube<f32>`, `texture_3d<f32>` or `texture_depth_2d`"
    )
}

fn wrong_effects(detail: &str) -> String {
    format!(
        "the shader must define \
         `fn effects(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> Outputs`, \
         with `struct Outputs` holding a `vec4<f32>` for each canvas: {detail}"
    )
}

/// The names of the members of the game's `struct Outputs`, in order.
fn output_names(module: &naga::Module) -> Result<Vec<String>, String> {
    let outputs = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Outputs"))
        .map(|(_, ty)| &ty.inner);
    let Some(naga::TypeInner::Struct { members, .. }) = outputs else {
        return Err(wrong_effects("`Outputs` must be a struct"));
    };
    if members.len() > smitten_core::backend::MAX_TARGETS {
        return Err(wrong_effects(&format!(
            "`Outputs` has {} members, but at most {} canvases can be drawn to at once",
            members.len(),
            smitten_core::backend::MAX_TARGETS
        )));
    }
    members
        .iter()
        .map(|member| {
            let is_vec4 = matches!(
                module.types[member.ty].inner,
                naga::TypeInner::Vector {
                    size: naga::VectorSize::Quad,
                    scalar: naga::Scalar::F32,
                }
            );
            let name = member.name.clone().unwrap_or_default();
            if is_vec4 {
                Ok(name)
            } else {
                Err(wrong_effects(&format!(
                    "`Outputs.{name}` is not a `vec4<f32>`"
                )))
            }
        })
        .collect()
}

/// Whether `source` defines a function called `name`, found as the
/// words `fn` and `name` in a row.
fn defines_function(source: &str, name: &str) -> bool {
    let mut words = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty());
    let mut previous = "";
    words.any(|word| {
        let found = previous == "fn" && word == name;
        previous = word;
        found
    })
}

fn wrong_effect(detail: &str) -> String {
    format!(
        "the shader must define \
         `fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32>`: {detail}"
    )
}

/// Whether `source` contains `struct` and `name` in a row, ignoring
/// comments well enough for the purpose: a mention in a comment only
/// costs an error message that says the struct is not defined.
fn declares_struct(source: &str, name: &str) -> bool {
    let mut words = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty());
    let mut previous = "";
    words.any(|word| {
        let found = previous == "struct" && word == name;
        previous = word;
        found
    })
}

/// Whether `source` has the word `name` anywhere.
fn mentions(source: &str, name: &str) -> bool {
    source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == name)
}

fn wrong_position(detail: &str) -> String {
    format!(
        "the shader must define \
         `fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32>`: \
         {detail}"
    )
}

/// Returns the size of `Params` and where each of its members lives.
fn params_layout(module: &naga::Module) -> Result<(u32, Vec<UniformField>), String> {
    let params = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some("Params"))
        .map(|(_, ty)| &ty.inner);
    let Some(naga::TypeInner::Struct { members, span }) = params else {
        return Err("`Params` must be a struct".to_string());
    };
    let fields = members
        .iter()
        .map(|member| {
            let (kind, array) = member_kind(module, &module.types[member.ty].inner);
            UniformField {
                name: member.name.clone().unwrap_or_default(),
                offset: member.offset,
                kind,
                array,
            }
        })
        .collect();
    Ok((*span, fields))
}

/// The type of a member of `Params`, and its shape if it is an array of
/// a type that can be sent.
fn member_kind(
    module: &naga::Module,
    ty: &naga::TypeInner,
) -> (Option<UniformKind>, Option<UniformArray>) {
    match *ty {
        naga::TypeInner::Array {
            base,
            size: naga::ArraySize::Constant(count),
            stride,
        } => match uniform_kind(&module.types[base].inner) {
            Some(kind) => (
                Some(kind),
                Some(UniformArray {
                    count: count.get(),
                    stride,
                }),
            ),
            None => (None, None),
        },
        _ => (uniform_kind(ty), None),
    }
}

fn uniform_kind(ty: &naga::TypeInner) -> Option<UniformKind> {
    use naga::{ScalarKind, TypeInner, VectorSize};
    match *ty {
        TypeInner::Scalar(scalar) if scalar.width == 4 => match scalar.kind {
            ScalarKind::Float => Some(UniformKind::Float),
            ScalarKind::Sint => Some(UniformKind::Int),
            ScalarKind::Uint => Some(UniformKind::Uint),
            _ => None,
        },
        TypeInner::Vector { size, scalar } if scalar.width == 4 => {
            Some(match (scalar.kind, size) {
                (ScalarKind::Float, VectorSize::Bi) => UniformKind::Vec2,
                (ScalarKind::Float, VectorSize::Tri) => UniformKind::Vec3,
                (ScalarKind::Float, VectorSize::Quad) => UniformKind::Vec4,
                (ScalarKind::Sint, VectorSize::Bi) => UniformKind::IVec2,
                (ScalarKind::Sint, VectorSize::Tri) => UniformKind::IVec3,
                (ScalarKind::Sint, VectorSize::Quad) => UniformKind::IVec4,
                (ScalarKind::Uint, VectorSize::Bi) => UniformKind::UVec2,
                (ScalarKind::Uint, VectorSize::Tri) => UniformKind::UVec3,
                (ScalarKind::Uint, VectorSize::Quad) => UniformKind::UVec4,
                _ => return None,
            })
        }
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } if scalar.kind == ScalarKind::Float && scalar.width == 4 => match (columns, rows) {
            (VectorSize::Bi, VectorSize::Bi) => Some(UniformKind::Mat2),
            (VectorSize::Tri, VectorSize::Tri) => Some(UniformKind::Mat3),
            (VectorSize::Quad, VectorSize::Quad) => Some(UniformKind::Mat4),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tests below call these, which translate to both languages and
    // check that they agree on whether the shader works. They return the
    // SPIR-V result, so the error messages are checked once. On a Mac the
    // Metal compiler checks every MSL translation too.

    fn both<T: std::fmt::Debug>(
        spirv: Result<T, String>,
        msl: Result<T, String>,
        msl_code: impl Fn(&T) -> &ShaderCode,
    ) -> Result<T, String> {
        match (&spirv, &msl) {
            (Ok(_), Err(error)) => panic!("only the MSL translation failed: {error}"),
            (Err(error), Ok(_)) => panic!("only the SPIR-V translation failed: {error}"),
            (_, Ok(msl)) => metal_accepts(msl_code(msl)),
            _ => {}
        }
        spirv
    }

    fn compile(source: &str) -> Result<Compiled, String> {
        both(
            super::compile(source, ShaderLanguage::Spirv),
            super::compile(source, ShaderLanguage::Msl),
            |compiled| &compiled.code,
        )
    }

    fn compile_for(source: &str, targets: u32, layered: bool) -> Result<Compiled, String> {
        both(
            super::compile_for(source, targets, layered, ShaderLanguage::Spirv),
            super::compile_for(source, targets, layered, ShaderLanguage::Msl),
            |compiled| &compiled.code,
        )
    }

    fn compile_vertex(
        source: &str,
        layout: &VertexLayout,
        linearize: bool,
    ) -> Result<MeshVertexShader, String> {
        both(
            super::compile_vertex(source, layout, linearize, ShaderLanguage::Spirv),
            super::compile_vertex(source, layout, linearize, ShaderLanguage::Msl),
            |compiled| &compiled.code,
        )
    }

    fn check(source: &str) -> Result<(), String> {
        let spirv = super::check(source, ShaderLanguage::Spirv);
        let msl = super::check(source, ShaderLanguage::Msl);
        assert_eq!(spirv.is_ok(), msl.is_ok(), "{spirv:?} {msl:?}");
        spirv
    }

    fn mesh_vertex(layout: &VertexLayout, gamma_correct: bool) -> Result<MeshVertexShader, String> {
        both(
            super::mesh_vertex(layout, gamma_correct, ShaderLanguage::Spirv),
            super::mesh_vertex(layout, gamma_correct, ShaderLanguage::Msl),
            |compiled| &compiled.code,
        )
    }

    /// Checks that Metal's own compiler accepts `code`, on a Mac that has
    /// it. Elsewhere this does nothing. Xcode may leave the Metal
    /// toolchain to a separate download, so a Mac without it skips the
    /// check, unless `SMITTEN_REQUIRE_METAL` is set, as it is in CI.
    fn metal_accepts(code: &ShaderCode) {
        if !cfg!(target_os = "macos") {
            return;
        }
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "smitten-msl-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let source = std::env::temp_dir().join(format!("{name}.metal"));
        let output = std::env::temp_dir().join(format!("{name}.air"));
        std::fs::write(&source, &code.bytes).unwrap();
        let result = std::process::Command::new("xcrun")
            .args(["-sdk", "macosx", "metal", "-std=macos-metal2.1", "-c"])
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .output();
        let _ = std::fs::remove_file(&source);
        let _ = std::fs::remove_file(&output);
        let required = std::env::var_os("SMITTEN_REQUIRE_METAL").is_some();
        let result = match result {
            Ok(result) => result,
            Err(error) if !required => {
                eprintln!("skipping the Metal compiler check: {error}");
                return;
            }
            Err(error) => panic!("could not run the Metal compiler: {error}"),
        };
        let errors = String::from_utf8_lossy(&result.stderr);
        if result.status.success() {
            return;
        }
        let missing =
            errors.contains("Metal Toolchain") || errors.contains("unable to find utility");
        if missing && !required {
            eprintln!("skipping the Metal compiler check: {errors}");
            return;
        }
        panic!(
            "Metal rejected the translation:\n{errors}\n{}",
            String::from_utf8_lossy(&code.bytes)
        );
    }

    #[test]
    fn the_sprite_shaders_translate_for_metal() {
        let [vertex, fragment, array] = sprite_shaders(ShaderLanguage::Msl).unwrap();
        for code in [&vertex, &fragment, &array] {
            metal_accepts(code);
        }
        let vertex = String::from_utf8(vertex.bytes).unwrap();
        let fragment = String::from_utf8(fragment.bytes).unwrap();
        // Metal pairs what the stages pass by these names.
        for name in ["[[user(loc0)", "[[user(loc1)"] {
            assert!(vertex.contains(name), "{vertex}");
            assert!(fragment.contains(name), "{fragment}");
        }
        // SDL binds the vertex buffer itself, from slot 14, for
        // `[[stage_in]]`.
        assert!(vertex.contains("[[stage_in]]"), "{vertex}");
        assert!(vertex.contains("[[attribute(2)]]"), "{vertex}");
        assert!(vertex.contains("[[buffer(0)]]"), "{vertex}");
        assert!(
            String::from_utf8(array.bytes)
                .unwrap()
                .contains("[[buffer(0)]]")
        );
    }

    #[test]
    fn metal_slots_follow_the_order_sdl_binds_in() {
        let source = "
var noise: texture_2d<f32>;
var shadow: texture_depth_2d;
struct Params { amount: f32 }

fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    let height = textureSampleLevel(noise, noise_sampler, vertex_position.xy, 0.0).r;
    return transform_projection * vertex_position + vec4<f32>(0.0, height * params.amount, 0.0, 0.0);
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let lit = textureSampleCompare(shadow, shadow_sampler, uv, 0.5);
    return textureSample(image, image_sampler, uv) * textureSample(noise, noise_sampler, uv)
        * color * lit * params.amount;
}
";
        let fragment = super::compile(source, ShaderLanguage::Msl).unwrap().code;
        let text = String::from_utf8(fragment.bytes).unwrap();
        for slot in [
            "image [[texture(0)]]",
            "image_sampler [[sampler(0)]]",
            "noise [[texture(1)]]",
            "noise_sampler [[sampler(1)]]",
            "shadow [[texture(2)]]",
            "shadow_sampler [[sampler(2)]]",
            "params [[buffer(0)]]",
        ] {
            assert!(text.contains(slot), "{slot} in {text}");
        }
        let vertex =
            super::compile_vertex(source, &standard_layout(), false, ShaderLanguage::Msl).unwrap();
        let text = String::from_utf8(vertex.code.bytes).unwrap();
        for slot in [
            "noise [[texture(1)]]",
            "smitten_draw [[buffer(0)]]",
            "params [[buffer(1)]]",
        ] {
            assert!(text.contains(slot), "{slot} in {text}");
        }
    }

    const PLAIN: &str = "
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSample(image, image_sampler, uv) * color;
}
";

    fn layout(attributes: &[(&str, VertexDataType, u8)]) -> VertexLayout {
        let mut offset = 0;
        let attributes = attributes
            .iter()
            .map(|&(name, data_type, components)| {
                let attribute = smitten_core::backend::VertexLayoutAttribute {
                    name: name.to_string(),
                    buffer: 0,
                    offset,
                    data_type,
                    components,
                };
                offset += data_type.size(components);
                attribute
            })
            .collect();
        VertexLayout {
            buffers: vec![smitten_core::backend::VertexBufferLayout {
                stride: offset.max(4),
                step: smitten_core::backend::VertexStep::PerVertex,
            }],
            attributes,
        }
    }

    #[test]
    fn a_mesh_vertex_shader_reads_the_attributes_it_knows_in_order() {
        let shader = mesh_vertex(
            &layout(&[
                ("Weight", VertexDataType::Float, 1),
                (VERTEX_COLOR, VertexDataType::UNorm8, 3),
                (VERTEX_POSITION, VertexDataType::Float, 3),
            ]),
            false,
        )
        .unwrap();
        assert_eq!(shader.code.bytes[..4], 0x0723_0203u32.to_le_bytes());
        // Position first, then color; the weight is not read.
        assert_eq!(shader.locations, [2, 1]);
    }

    #[test]
    fn every_attribute_type_and_size_makes_a_valid_mesh_vertex_shader() {
        for data_type in [
            VertexDataType::Float,
            VertexDataType::UNorm8,
            VertexDataType::UNorm16,
        ] {
            for components in 1..=4 {
                let shader = mesh_vertex(
                    &layout(&[
                        (VERTEX_POSITION, data_type, components),
                        (VERTEX_TEX_COORD, data_type, components),
                        (VERTEX_COLOR, data_type, components),
                    ]),
                    components == 4,
                )
                .unwrap_or_else(|error| panic!("{data_type:?} x{components}: {error}"));
                assert_eq!(shader.locations, [0, 1, 2]);
            }
        }
        // Even with none of them.
        assert!(
            mesh_vertex(&layout(&[]), true)
                .unwrap()
                .locations
                .is_empty()
        );
    }

    #[test]
    fn effects_writes_each_output_to_its_own_canvas() {
        let source = "
struct Outputs { albedo: vec4<f32>, normal: vec4<f32> }

fn effects(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> Outputs {
    var out: Outputs;
    out.albedo = textureSample(image, image_sampler, uv) * color;
    out.normal = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    return out;
}
";
        for targets in 1..=4 {
            compile_for(source, targets, false).unwrap();
        }
        let error = compile_for(
            "
struct Outputs { albedo: vec3<f32> }
fn effects(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> Outputs {
    var out: Outputs;
    return out;
}
",
            2,
            false,
        )
        .unwrap_err();
        assert!(
            error.contains("`Outputs.albedo` is not a `vec4<f32>`"),
            "{error}"
        );
    }

    #[test]
    fn the_built_in_shaders_compile_for_several_canvases() {
        for targets in 1..=4 {
            compile_for(PLAIN, targets, false).unwrap();
            compile_for(BUILTIN, targets, false).unwrap();
            let array = compile_for(BUILTIN_ARRAY, targets, true).unwrap();
            assert_eq!(array.uniform_size, 4);
        }
    }

    #[test]
    fn a_plain_effect_compiles_to_spirv() {
        let compiled = compile(PLAIN).unwrap();
        // Every SPIR-V module starts with this number.
        assert_eq!(compiled.code.bytes[..4], 0x0723_0203u32.to_le_bytes());
        assert_eq!(compiled.uniform_size, 0);
        assert!(compiled.fields.is_empty());
    }

    #[test]
    fn params_are_laid_out_by_the_shader_languages_rules() {
        let compiled = compile(
            "
struct Params {
    time: f32,
    tint: vec4<f32>,
    center: vec2<f32>,
    steps: i32,
    light: vec3<f32>,
    count: u32,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * params.tint * params.time;
}
",
        )
        .unwrap();

        let fields: Vec<_> = compiled
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset, field.kind))
            .collect();
        assert_eq!(
            fields,
            [
                ("time", 0, Some(UniformKind::Float)),
                // A vec4 starts on a multiple of 16 bytes.
                ("tint", 16, Some(UniformKind::Vec4)),
                ("center", 32, Some(UniformKind::Vec2)),
                ("steps", 40, Some(UniformKind::Int)),
                ("light", 48, Some(UniformKind::Vec3)),
                // A u32 fits in the space after a vec3.
                ("count", 60, Some(UniformKind::Uint)),
            ]
        );
        assert_eq!(compiled.uniform_size, 64);
    }

    #[test]
    fn the_games_own_textures_get_samplers_and_slots_in_order() {
        let compiled = compile(
            "
var sky: texture_cube<f32>;
var tiles: texture_2d_array<f32>;
var noise: texture_2d<f32>;
var fog: texture_3d<f32>;

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let a = textureSample(sky, sky_sampler, vec3<f32>(uv, 1.0));
    let b = textureSample(tiles, tiles_sampler, uv, 2);
    let c = textureSample(noise, noise_sampler, uv);
    let d = textureSample(fog, fog_sampler, vec3<f32>(uv, 0.5));
    return a * b * c * d * color;
}
",
        )
        .unwrap();
        let textures: Vec<_> = compiled
            .textures
            .iter()
            .map(|texture| (texture.name.as_str(), texture.kind))
            .collect();
        assert_eq!(
            textures,
            [
                ("sky", TextureKind::Cube),
                ("tiles", TextureKind::Array),
                ("noise", TextureKind::Flat),
                ("fog", TextureKind::Volume),
            ]
        );
        assert!(compile(PLAIN).unwrap().textures.is_empty());
    }

    #[test]
    fn a_depth_texture_is_sampled_by_comparing() {
        let compiled = compile(
            "
var shadow: texture_depth_2d;
var noise: texture_2d<f32>;

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let lit = textureSampleCompare(shadow, shadow_sampler, uv, 0.5);
    return textureSample(noise, noise_sampler, uv) * lit * color;
}
",
        )
        .unwrap();
        let textures: Vec<_> = compiled
            .textures
            .iter()
            .map(|texture| (texture.name.as_str(), texture.kind, texture.comparison))
            .collect();
        assert_eq!(
            textures,
            [
                ("shadow", TextureKind::Flat, true),
                ("noise", TextureKind::Flat, false),
            ]
        );
    }

    #[test]
    fn a_texture_of_another_type_is_explained() {
        let error = compile(
            "
var line: texture_1d<f32>;
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSample(line, line_sampler, uv.x) * color;
}
",
        )
        .unwrap_err();
        assert!(error.contains("texture `line` must be"), "{error}");
    }

    #[test]
    fn a_type_that_cannot_be_sent_is_still_listed() {
        let compiled = compile(
            "
struct Light { power: f32 }
struct Params { light: Light, turn: mat2x3<f32> }

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * params.light.power * params.turn[0].x;
}
",
        )
        .unwrap();
        assert_eq!(compiled.fields[0].name, "light");
        assert_eq!(compiled.fields[0].kind, None);
        assert_eq!(compiled.fields[1].kind, None);
    }

    #[test]
    fn matrices_integer_vectors_and_arrays_are_laid_out() {
        let compiled = compile(
            "
struct Params {
    turn: mat2x2<f32>,
    tile: vec2<i32>,
    view: mat3x3<f32>,
    world: mat4x4<f32>,
    lights: array<vec4<f32>, 8>,
    bones: array<mat4x4<f32>, 2>,
    flags: vec4<u32>,
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * params.turn[0].x * params.view[0].x * params.world[0].x
        * params.lights[2] * params.bones[1][0] * f32(params.tile.x + i32(params.flags.y));
}
",
        )
        .unwrap();
        let fields: Vec<_> = compiled
            .fields
            .iter()
            .map(|field| (field.name.as_str(), field.offset, field.kind, field.array))
            .collect();
        let array = |count, stride| Some(UniformArray { count, stride });
        assert_eq!(
            fields,
            [
                ("turn", 0, Some(UniformKind::Mat2), None),
                ("tile", 16, Some(UniformKind::IVec2), None),
                ("view", 32, Some(UniformKind::Mat3), None),
                ("world", 80, Some(UniformKind::Mat4), None),
                ("lights", 144, Some(UniformKind::Vec4), array(8, 16)),
                ("bones", 272, Some(UniformKind::Mat4), array(2, 64)),
                ("flags", 400, Some(UniformKind::UVec4), None),
            ]
        );
    }

    #[test]
    fn a_syntax_error_names_the_line_in_the_games_source() {
        let error = compile(
            "fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color *;
}",
        )
        .unwrap_err();
        assert!(error.contains("shader:2:"), "{error}");
    }

    #[test]
    fn a_missing_effect_function_is_explained() {
        let error = compile("fn helper() -> f32 { return 1.0; }").unwrap_err();
        assert!(error.contains("must define `fn effect("), "{error}");
    }

    #[test]
    fn an_effect_with_the_wrong_arguments_is_explained() {
        let error =
            compile("fn effect(color: vec4<f32>) -> vec4<f32> { return color; }").unwrap_err();
        assert!(error.contains("must define `fn effect("), "{error}");
    }

    const WAVE: &str = "
struct Params { time: f32 }

struct Attributes {
    Offset: vec2<f32>,
    Missing: f32,
}

struct Varyings {
    height: f32,
    tile: vec2<i32>,
}

fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    varyings.height = vertex_position.y + attributes.Missing;
    varyings.tile = vec2<i32>(attributes.Offset);
    let moved = vertex_position + vec4<f32>(attributes.Offset, 0.0, 0.0);
    return transform_projection * (moved + vec4<f32>(0.0, sin(params.time), 0.0, 0.0));
}

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * varyings.height + vec4<f32>(f32(varyings.tile.x));
}
";

    fn offset_layout() -> VertexLayout {
        let mut layout = standard_layout();
        layout.buffers.push(VertexBufferLayout {
            stride: 8,
            step: VertexStep::PerInstance,
        });
        layout.attributes.push(VertexLayoutAttribute {
            name: "Offset".to_string(),
            buffer: 1,
            offset: 0,
            data_type: VertexDataType::Float,
            components: 2,
        });
        layout
    }

    #[test]
    fn a_vertex_shader_compiles_for_the_standard_layout_and_a_mesh_layout() {
        let compiled = compile(WAVE).unwrap();
        assert!(compiled.has_vertex);
        assert_eq!(compiled.uniform_size, 4);
        let standard = compile_vertex(WAVE, &standard_layout(), false).unwrap();
        // Offset is missing from the standard layout, so it is not read.
        assert_eq!(standard.locations, [0, 1, 2]);
        let mesh = compile_vertex(WAVE, &offset_layout(), true).unwrap();
        // Offset is read first, as the attributes are filled first.
        assert_eq!(mesh.locations, [3, 0, 1, 2]);
        check(WAVE).unwrap();
    }

    #[test]
    fn a_shader_with_only_a_position_gets_the_built_in_effect() {
        let source = "
fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    return transform_projection * vertex_position;
}
";
        let compiled = compile(source).unwrap();
        assert!(compiled.has_vertex);
        compile_vertex(source, &standard_layout(), false).unwrap();
        // It can be drawn to several canvases too.
        compile_for(source, 3, false).unwrap();
    }

    #[test]
    fn a_shader_without_a_position_has_no_vertex_stage() {
        assert!(!compile(BUILTIN).unwrap().has_vertex);
        check(BUILTIN).unwrap();
    }

    #[test]
    fn varyings_without_a_position_are_explained() {
        let error = compile(
            "struct Varyings { height: f32 }
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * varyings.height;
}",
        )
        .unwrap_err();
        assert!(error.contains("must define `fn position("), "{error}");
    }

    #[test]
    fn a_position_with_the_wrong_arguments_is_explained() {
        let error = check(
            "fn position(vertex_position: vec4<f32>) -> vec4<f32> { return vertex_position; }",
        )
        .unwrap_err();
        assert!(error.contains("must define `fn position("), "{error}");
    }

    #[test]
    fn unusable_attributes_and_varyings_are_explained() {
        let position = "
fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    return transform_projection * vertex_position;
}
";
        let attributes = format!("struct Attributes {{ count: u32 }}\n{position}");
        let error = check(&attributes).unwrap_err();
        assert!(error.contains("`Attributes.count` must be"), "{error}");
        let varyings = format!("struct Varyings {{ turn: mat2x2<f32> }}\n{position}");
        let error = check(&varyings).unwrap_err();
        assert!(error.contains("`Varyings.turn` must be"), "{error}");
    }

    #[test]
    fn a_vertex_shader_can_sample_the_games_textures() {
        let source = "
var height_map: texture_2d<f32>;

fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    let height = textureSampleLevel(height_map, height_map_sampler, vertex_position.xy, 0.0).r;
    return transform_projection * (vertex_position + vec4<f32>(0.0, height, 0.0, 0.0));
}
";
        assert_eq!(compile(source).unwrap().textures.len(), 1);
        check(source).unwrap();
    }

    #[test]
    fn instance_index_counts_the_copies_drawn() {
        let source = "
fn position(transform_projection: mat4x4<f32>, vertex_position: vec4<f32>) -> vec4<f32> {
    let step = vec4<f32>(f32(instance_index) * 20.0, 0.0, 0.0, 0.0);
    return transform_projection * (vertex_position + step);
}
";
        check(source).unwrap();
        let compiled = compile_vertex(source, &offset_layout(), false).unwrap();
        // The built-in index is not a vertex attribute.
        assert_eq!(compiled.locations, [0, 1, 2]);
    }
}
