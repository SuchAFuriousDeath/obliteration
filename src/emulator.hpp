// This file is an interface to emulator/src/lib.rs.
#pragma once

#include <cinttypes>

struct emulator_config {
};

extern "C" {
    void *emulator_init(char **error);
    void emulator_term(void *inst);

    char *emulator_start(const emulator_config *conf);
}