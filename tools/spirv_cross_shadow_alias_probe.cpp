#include "spirv_msl.hpp"

#include <cstdint>
#include <cstring>
#include <fstream>
#include <iostream>
#include <iterator>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

std::vector<uint32_t> read_spirv(const std::string& path) {
    std::ifstream input(path, std::ios::binary);
    if (!input) {
        throw std::runtime_error("cannot open SPIR-V input: " + path);
    }

    const std::vector<char> bytes((std::istreambuf_iterator<char>(input)),
                                  std::istreambuf_iterator<char>());
    if (bytes.empty() || bytes.size() % sizeof(uint32_t) != 0) {
        throw std::runtime_error("SPIR-V input has an invalid byte length");
    }

    std::vector<uint32_t> words(bytes.size() / sizeof(uint32_t));
    std::memcpy(words.data(), bytes.data(), bytes.size());
    return words;
}

void add_sampled_image_binding(spirv_cross::CompilerMSL& compiler,
                               uint32_t binding,
                               uint32_t texture_index,
                               uint32_t sampler_index) {
    spirv_cross::MSLResourceBinding resource;
    resource.stage = spv::ExecutionModelFragment;
    resource.basetype = spirv_cross::SPIRType::SampledImage;
    resource.desc_set = 0;
    resource.binding = binding;
    resource.count = 1;
    resource.msl_texture = texture_index;
    resource.msl_sampler = sampler_index;
    compiler.add_msl_resource_binding(resource);
}

} // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::cerr << "usage: spirv-cross-shadow-alias-probe SHADER.spv\n";
        return 2;
    }

    try {
        spirv_cross::CompilerMSL compiler(read_spirv(argv[1]));
        spirv_cross::CompilerMSL::Options options;
        options.platform = spirv_cross::CompilerMSL::Options::macOS;
        options.set_msl_version(3, 0, 0);
        options.argument_buffers = true;
        options.argument_buffers_tier =
            spirv_cross::CompilerMSL::Options::ArgumentBuffersTier::Tier2;
        compiler.set_msl_options(options);

        add_sampled_image_binding(compiler, 4, 5, 6);
        add_sampled_image_binding(compiler, 5, 7, 8);
        add_sampled_image_binding(compiler, 6, 9, 10);

        std::cout << compiler.compile();
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
