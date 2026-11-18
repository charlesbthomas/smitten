//! Turns a game's WGSL effect into a fragment shader SDL can load.
//!
//! The game writes one function. This module wraps it in an entry point
//! with the texture, sampler and uniform bindings SDL's GPU API expects,
//! checks it, and translates it to SPIR-V with naga.
//!
//! A game may also declare textures of its own, such as
//! `var sky: texture_cube<f32>;` or `var fog: texture_3d<f32>;`, with no binding. Each gets a sampler
//! named after it, `sky_sampler`, and the next free texture slot.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use smitten_core::backend::{
    ShaderTexture, TextureKind, UniformArray, UniformField, UniformKind, VERTEX_COLOR,
    VERTEX_POSITION, VERTEX_TEX_COORD, VertexDataType, VertexLayout,
};

/// The name of the entry point in the generated shader.
pub(crate) const ENTRY_POINT: &std::ffi::CStr = c"smitten_fragment";

/// Appended after the game's source, so its line numbers stay the same in
/// error messages. WGSL does not care about declaration order. `IMAGE` is
/// replaced with the type of the texture being drawn.
///
/// SDL wants fragment shader textures in group 2 and uniforms in group 3.
const WRAPPER: &str = "
@group(2) @binding(0) var image: IMAGE;
@group(2) @binding(1) var image_sampler: sampler;

struct SmittenFragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}
";

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
fn single_entry(targets: u32) -> String {
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
    let result = effect(input.color, input.uv, input.position.xy);
    var output: SmittenFragmentOutput;
{copies}    return output;
}}
"
    )
}

/// The entry point for a shader with several outputs, `effects`, whose
/// `Outputs` struct has the members `outputs`, drawing to `targets`
/// canvases. A canvas past the last member gets transparent black.
fn multiple_entry(outputs: &[String], targets: u32) -> String {
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
    let result: Outputs = effects(input.color, input.uv, input.position.xy);
    var output: SmittenFragmentOutput;
{copies}    return output;
}}
"
    )
}

/// The name of the entry point of the built-in mesh vertex shader.
pub(crate) const MESH_VERTEX_ENTRY_POINT: &std::ffi::CStr = c"smitten_vertex";

/// The part of every mesh vertex shader that does not depend on the
/// layout. It places each vertex with the draw's matrix and tints its
/// color, where ordinary draws are placed and tinted on the CPU. Its
/// outputs match what every fragment shader takes. SDL wants vertex shader
/// uniforms in group 1.
const MESH_VERTEX_COMMON: &str = "
struct SmittenDraw {
    transform_projection: mat4x4<f32>,
    tint: vec4<f32>,
}

@group(1) @binding(0) var<uniform> smitten_draw: SmittenDraw;

struct SmittenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}
";

/// A vertex shader made for one vertex layout.
#[derive(Debug)]
pub(crate) struct MeshVertexShader {
    pub spirv: Vec<u8>,
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
    format!("vec{}<f32>({})", defaults.len(), values.join(", "))
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
/// position, texture coordinate and color by their LÖVE names, with
/// LÖVE's defaults for any the layout lacks. With `gamma_correct`, vertex
/// colors are turned into linear light, as LÖVE does.
pub(crate) fn mesh_vertex(
    layout: &VertexLayout,
    gamma_correct: bool,
) -> Result<MeshVertexShader, String> {
    let mut source = String::from(MESH_VERTEX_COMMON);
    source.push_str(LINEAR_COLOR);
    let mut parameters = Vec::new();
    let mut locations = Vec::new();
    let mut read = |name: &str, defaults: &[f32]| {
        let found = layout
            .attributes
            .iter()
            .enumerate()
            .find(|(_, attribute)| attribute.name == name);
        match found {
            Some((index, attribute)) => {
                let (ty, width) = attribute_shader_type(attribute.data_type, attribute.components);
                let variable = format!("in_{}", locations.len());
                parameters.push(format!("@location({}) {variable}: {ty}", locations.len()));
                locations.push(index);
                widen(&variable, width, attribute.components, defaults)
            }
            None => widen("", 0, 0, defaults),
        }
    };
    let position = read(VERTEX_POSITION, &[0.0, 0.0, 0.0, 1.0]);
    let uv = read(VERTEX_TEX_COORD, &[0.0, 0.0]);
    let color = read(VERTEX_COLOR, &[1.0, 1.0, 1.0, 1.0]);
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
        parameters.join(", ")
    ));
    let spirv = vertex_spirv(&source)?;
    Ok(MeshVertexShader { spirv, locations })
}

/// Translates a vertex shader the framework wrote to SPIR-V.
fn vertex_spirv(source: &str) -> Result<Vec<u8>, String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|error| error.emit_to_string_with_path(source, "mesh vertex shader"))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| error.emit_to_string_with_path(source, "mesh vertex shader"))?;
    let pipeline = spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Vertex,
        entry_point: "smitten_vertex".to_string(),
    };
    // SDL's clip space already has y up, as WGSL's does, so naga must not
    // flip it the way it would for plain Vulkan.
    let mut options = spv::Options::default();
    options
        .flags
        .remove(spv::WriterFlags::ADJUST_COORDINATE_SPACE);
    let words = spv::write_vec(&module, &info, &options, Some(&pipeline))
        .map_err(|error| format!("could not translate the mesh vertex shader: {error}"))?;
    Ok(words.iter().flat_map(|word| word.to_le_bytes()).collect())
}

/// Added only when the game declares `struct Params`.
const PARAMS_BINDING: &str = "
@group(3) @binding(0) var<uniform> params: Params;
";

/// A fragment shader ready to hand to SDL.
#[derive(Debug)]
pub(crate) struct Compiled {
    pub spirv: Vec<u8>,
    /// The size of the `Params` struct in bytes, or 0 without one.
    pub uniform_size: u32,
    pub fields: Vec<UniformField>,
    /// The game's own textures, in the order of their slots after `image`.
    pub textures: Vec<ShaderTexture>,
}

/// Compiles a game's shader to draw to one canvas.
pub(crate) fn compile(source: &str) -> Result<Compiled, String> {
    compile_for(source, 1, false)
}

/// Compiles a game's shader, or one of the built-in ones, to draw to
/// `targets` canvases at once. With `layered`, the texture drawn is an
/// array texture.
pub(crate) fn compile_for(source: &str, targets: u32, layered: bool) -> Result<Compiled, String> {
    let has_params = declares_params(source);
    let multiple = defines_function(source, "effects");
    let texture_names = declared_textures(source);
    let mut full = String::from(source);
    full.push_str(&WRAPPER.replace(
        "IMAGE",
        if layered {
            "texture_2d_array<f32>"
        } else {
            "texture_2d<f32>"
        },
    ));
    for name in &texture_names {
        full.push_str(&format!("var {name}_sampler: sampler;\n"));
    }
    if has_params {
        full.push_str(PARAMS_BINDING);
    }
    let user_lines = source.lines().count() as u32;
    let wrong = if multiple {
        wrong_effects
    } else {
        wrong_effect
    };
    let parse = |full: &str| {
        naga::front::wgsl::parse_str(full).map_err(|error| {
            // An error placed in the wrapper means it did not fit the
            // game's `effect`, which is the game's mistake to fix, not
            // ours to show.
            match error.location(full) {
                Some(location) if location.line_number > user_lines => wrong(error.message()),
                _ => error.emit_to_string_with_path(full, "shader"),
            }
        })
    };
    // The entry point copies the members of `Outputs`, so their names
    // come from parsing the source without it first.
    let entry = if multiple {
        multiple_entry(&output_names(&parse(&full)?)?, targets)
    } else {
        single_entry(targets)
    };
    full.push_str(&entry);

    let mut module = parse(&full)?;
    let textures = bind_textures(&mut module, &texture_names)?;
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

    let (uniform_size, fields) = if has_params {
        params_layout(&module)?
    } else {
        (0, Vec::new())
    };

    // WGSL has a texture and a sampler as two things; SDL binds them as
    // one. Vulkan lets both variables name the same combined binding.
    let mut options = spv::Options::default();
    options.binding_map.insert(
        naga::ResourceBinding {
            group: 2,
            binding: 1,
        },
        spv::BindingInfo {
            descriptor_set: 2,
            binding: 0,
            binding_array_size: None,
        },
    );
    for slot in 1..=textures.len() as u32 {
        for binding in [slot * 2, slot * 2 + 1] {
            options.binding_map.insert(
                naga::ResourceBinding { group: 2, binding },
                spv::BindingInfo {
                    descriptor_set: 2,
                    binding: slot,
                    binding_array_size: None,
                },
            );
        }
    }
    let pipeline = spv::PipelineOptions {
        shader_stage: naga::ShaderStage::Fragment,
        entry_point: "smitten_fragment".to_string(),
    };
    let words = spv::write_vec(&module, &info, &options, Some(&pipeline))
        .map_err(|error| format!("could not translate the shader: {error}"))?;
    let spirv = words.iter().flat_map(|word| word.to_le_bytes()).collect();

    Ok(Compiled {
        spirv,
        uniform_size,
        fields,
        textures,
    })
}

/// The names of the textures `source` declares at the top level with no
/// binding, in order, found well enough for the purpose: `var`, a name,
/// then a texture type. A declaration with a binding is left alone, and
/// one the scan misses fails to compile with a message about bindings.
fn declared_textures(source: &str) -> Vec<String> {
    let words: Vec<&str> = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let mut names: Vec<String> = Vec::new();
    for window in words.windows(3) {
        if window[0] == "var"
            && window[2].starts_with("texture_")
            && !names.iter().any(|name| name == window[1])
        {
            names.push(window[1].to_string());
        }
    }
    names
}

/// Gives each of the game's own textures in `names`, and its sampler,
/// the slot after the ones before it. Returns what kind each one is.
fn bind_textures(
    module: &mut naga::Module,
    names: &[String],
) -> Result<Vec<ShaderTexture>, String> {
    let mut textures = Vec::new();
    for name in names {
        let found = module
            .global_variables
            .iter()
            .find(|(_, global)| global.name.as_deref() == Some(name) && global.binding.is_none())
            .map(|(handle, global)| (handle, global.ty));
        let Some((handle, ty)) = found else {
            continue;
        };
        let kind = match module.types[ty].inner {
            naga::TypeInner::Image {
                dim,
                arrayed,
                class:
                    naga::ImageClass::Sampled {
                        kind: naga::ScalarKind::Float,
                        multi: false,
                    },
            } => match (dim, arrayed) {
                (naga::ImageDimension::D2, false) => TextureKind::Flat,
                (naga::ImageDimension::D2, true) => TextureKind::Array,
                (naga::ImageDimension::Cube, false) => TextureKind::Cube,
                (naga::ImageDimension::D3, false) => TextureKind::Volume,
                _ => return Err(unsupported_texture(name)),
            },
            _ => return Err(unsupported_texture(name)),
        };
        let slot = textures.len() as u32 + 1;
        module.global_variables.get_mut(handle).binding = Some(naga::ResourceBinding {
            group: 2,
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
                group: 2,
                binding: slot * 2 + 1,
            });
        }
        textures.push(ShaderTexture {
            name: name.clone(),
            kind,
        });
    }
    Ok(textures)
}

fn unsupported_texture(name: &str) -> String {
    format!(
        "texture `{name}` must be a `texture_2d<f32>`, `texture_2d_array<f32>`, \
         `texture_cube<f32>` or `texture_3d<f32>`"
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

/// Whether `source` contains `struct Params`, ignoring comments well
/// enough for the purpose: a mention in a comment only costs an error
/// message that says `Params` is not defined.
fn declares_params(source: &str) -> bool {
    let mut words = source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty());
    let mut previous = "";
    words.any(|word| {
        let found = previous == "struct" && word == "Params";
        previous = word;
        found
    })
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
        assert_eq!(shader.spirv[..4], 0x0723_0203u32.to_le_bytes());
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
        assert_eq!(compiled.spirv[..4], 0x0723_0203u32.to_le_bytes());
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
}
