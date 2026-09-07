#include "memory_reader_bridge.hpp"

#include <chrono>
#include <memory>
#include <vector>

#include <yourgg/memory_reader.hpp>

namespace {

struct ReaderHandle {
    ReaderHandle()
        : reader([] {
              yourgg::lol::ReaderConfig config;
              config.pollingInterval = std::chrono::milliseconds{500};
              return config;
          }()) {}

    yourgg::lol::AugmentReader reader;
};

}  // namespace

extern "C" void* yourgg_augment_reader_create() {
    try {
        return new ReaderHandle{};
    } catch (...) {
        return nullptr;
    }
}

extern "C" bool yourgg_augment_reader_start(
    void* handle,
    void* context,
    YourggAugmentChoicesCallback on_choices,
    YourggAugmentErrorCallback on_error) {
    if (!handle || !on_choices) {
        return false;
    }

    try {
        auto* state = static_cast<ReaderHandle*>(handle);
        return state->reader.start(
            [context, on_choices](const yourgg::lol::AugmentChoices& choices) {
                std::vector<YourggAugmentCard> cards;
                cards.reserve(choices.cards.size());
                for (const auto& card : choices.cards) {
                    cards.push_back({
                        .instance = card.instance,
                        .name = card.name.c_str(),
                        .description = card.description.c_str(),
                        .x = card.x,
                        .y = card.y,
                        .width = card.width,
                        .height = card.height,
                    });
                }
                on_choices(
                    context,
                    yourgg::lol::toString(choices.mode),
                    cards.data(),
                    cards.size());
            },
            [context, on_error](yourgg::lol::ReadError error) {
                if (on_error) {
                    on_error(context, yourgg::lol::toString(error));
                }
            });
    } catch (...) {
        return false;
    }
}

extern "C" void yourgg_augment_reader_stop(void* handle) {
    if (handle) {
        static_cast<ReaderHandle*>(handle)->reader.stop();
    }
}

extern "C" void yourgg_augment_reader_destroy(void* handle) {
    delete static_cast<ReaderHandle*>(handle);
}
