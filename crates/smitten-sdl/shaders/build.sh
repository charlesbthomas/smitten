#!/bin/sh
# Recompiles the built-in shaders to SPIR-V. Run this after editing a shader
# and commit the .spv files, so building Smitten needs no shader compiler.
# Requires glslangValidator (package glslang-tools, or the Vulkan SDK).
set -eu
cd "$(dirname "$0")"
glslangValidator -V sprite.vert -o sprite.vert.spv
glslangValidator -V sprite.frag -o sprite.frag.spv
