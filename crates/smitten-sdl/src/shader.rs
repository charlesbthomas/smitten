//! Turns a game's WGSL effect into a fragment shader SDL can load.
//!
//! The game writes one function. This module wraps it in an entry point
//! with the texture, sampler and uniform bindings SDL's GPU API expects,
//! checks it, and translates it to SPIR-V with naga.
//!
//! A game may also declare textures of its own, such as
//! `var sky: texture_cube<f32>;`, with no binding. Each gets a sampler
//! named after it, `sky_sampler`, and the next free texture slot.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use smitten_core::backend::{ShaderTexture, TextureKind, UniformArray, UniformField, UniformKind};

/// The name of the entry point in the generated shader.
pub(crate) const ENTRY_POINT: &std::ffi::CStr = c"smitten_fragment";

/// Appended after the game's source, so its line numbers stay the same in
/// error messages. WGSL does not care about declaration order.
///
/// SDL wants fragment shader textures in group 2 and uniforms in group 3.
const WRAPPER: &str = "
@group(2) @binding(0) var image: texture_2d<f32>;
@group(2) @binding(1) var image_sampler: sampler;

struct SmittenFragmentInput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@fragment
fn smitten_fragment(input: SmittenFragmentInput) -> @location(0) vec4<f32> {
    return effect(input.color, input.uv, input.position.xy);
}
";

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

pub(crate) fn compile(source: &str) -> Result<Compiled, String> {
    let has_params = declares_params(source);
    let texture_names = declared_textures(source);
    let mut full = String::from(source);
    full.push_str(WRAPPER);
    for name in &texture_names {
        full.push_str(&format!("var {name}_sampler: sampler;\n"));
    }
    if has_params {
        full.push_str(PARAMS_BINDING);
    }
    let user_lines = source.lines().count() as u32;

    let mut module = naga::front::wgsl::parse_str(&full).map_err(|error| {
        // An error placed in the wrapper means it did not fit the game's
        // `effect`, which is the game's mistake to fix, not ours to show.
        match error.location(&full) {
            Some(location) if location.line_number > user_lines => wrong_effect(error.message()),
            _ => error.emit_to_string_with_path(&full, "shader"),
        }
    })?;
    let textures = bind_textures(&mut module, &texture_names)?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .map_err(|error| {
            let report = error.emit_to_string_with_path(&full, "shader");
            if report.contains("smitten_fragment") {
                wrong_effect("its arguments or return type do not match")
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
        "texture `{name}` must be a `texture_2d<f32>`, `texture_2d_array<f32>` or `texture_cube<f32>`"
    )
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

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    let a = textureSample(sky, sky_sampler, vec3<f32>(uv, 1.0));
    let b = textureSample(tiles, tiles_sampler, uv, 2);
    let c = textureSample(noise, noise_sampler, uv);
    return a * b * c * color;
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
            ]
        );
        assert!(compile(PLAIN).unwrap().textures.is_empty());
    }

    #[test]
    fn a_texture_of_another_type_is_explained() {
        let error = compile(
            "
var depth: texture_3d<f32>;
fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return textureSample(depth, depth_sampler, vec3<f32>(uv, 0.0)) * color;
}
",
        )
        .unwrap_err();
        assert!(error.contains("texture `depth` must be"), "{error}");
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
