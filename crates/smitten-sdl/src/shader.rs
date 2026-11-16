//! Turns a game's WGSL effect into a fragment shader SDL can load.
//!
//! The game writes one function. This module wraps it in an entry point
//! with the texture, sampler and uniform bindings SDL's GPU API expects,
//! checks it, and translates it to SPIR-V with naga.

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use smitten_core::backend::{UniformField, UniformKind};

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
}

pub(crate) fn compile(source: &str) -> Result<Compiled, String> {
    let has_params = declares_params(source);
    let mut full = String::from(source);
    full.push_str(WRAPPER);
    if has_params {
        full.push_str(PARAMS_BINDING);
    }
    let user_lines = source.lines().count() as u32;

    let module = naga::front::wgsl::parse_str(&full).map_err(|error| {
        // An error placed in the wrapper means it did not fit the game's
        // `effect`, which is the game's mistake to fix, not ours to show.
        match error.location(&full) {
            Some(location) if location.line_number > user_lines => wrong_effect(error.message()),
            _ => error.emit_to_string_with_path(&full, "shader"),
        }
    })?;
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
        .map(|member| UniformField {
            name: member.name.clone().unwrap_or_default(),
            offset: member.offset,
            kind: uniform_kind(&module.types[member.ty].inner),
        })
        .collect();
    Ok((*span, fields))
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
        TypeInner::Vector { size, scalar }
            if scalar.kind == ScalarKind::Float && scalar.width == 4 =>
        {
            Some(match size {
                VectorSize::Bi => UniformKind::Vec2,
                VectorSize::Tri => UniformKind::Vec3,
                VectorSize::Quad => UniformKind::Vec4,
            })
        }
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
    fn a_type_that_cannot_be_sent_is_still_listed() {
        let compiled = compile(
            "
struct Params { turn: mat2x2<f32> }

fn effect(color: vec4<f32>, uv: vec2<f32>, pixel: vec2<f32>) -> vec4<f32> {
    return color * params.turn[0].x;
}
",
        )
        .unwrap();
        assert_eq!(compiled.fields[0].name, "turn");
        assert_eq!(compiled.fields[0].kind, None);
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
