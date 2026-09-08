#pragma once

#include <cstddef>
#include <cstdint>

extern "C" {

struct YourggAugmentCard {
    std::uint32_t instance;
    const char* name;
    const char* description;
    float x;
    float y;
    float width;
    float height;
};

using YourggAugmentChoicesCallback = void (*)(
    void* context,
    const char* mode,
    const YourggAugmentCard* cards,
    std::size_t card_count);
using YourggAugmentOwnedCallback = void (*)(
    void* context,
    const char* const* names,
    std::size_t name_count);
using YourggAugmentErrorCallback = void (*)(void* context, const char* error);

void* yourgg_augment_reader_create();
bool yourgg_augment_reader_start(
    void* handle,
    void* context,
    YourggAugmentChoicesCallback on_choices,
    YourggAugmentOwnedCallback on_owned,
    YourggAugmentErrorCallback on_error);
void yourgg_augment_reader_stop(void* handle);
void yourgg_augment_reader_destroy(void* handle);

}
