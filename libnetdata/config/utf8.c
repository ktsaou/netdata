// SPDX-License-Identifier: GPL-3.0-or-later

#include "../libnetdata.h"

static inline bool encode_utf8(unsigned codepoint, char **d, size_t *remaining) {
    if (codepoint <= 0x7F) {
        // 1-byte sequence
        if (*remaining < 2) return false; // +1 for the null
        *(*d)++ = (char)codepoint;
        (*remaining)--;
    }
    else if (codepoint <= 0x7FF) {
        // 2-byte sequence
        if (*remaining < 3) return false; // +1 for the null
        *(*d)++ = (char)(0xC0 | ((codepoint >> 6) & 0x1F));
        *(*d)++ = (char)(0x80 | (codepoint & 0x3F));
        (*remaining) -= 2;
    }
    else if (codepoint <= 0xFFFF) {
        // 3-byte sequence
        if (*remaining < 4) return false; // +1 for the null
        *(*d)++ = (char)(0xE0 | ((codepoint >> 12) & 0x0F));
        *(*d)++ = (char)(0x80 | ((codepoint >> 6) & 0x3F));
        *(*d)++ = (char)(0x80 | (codepoint & 0x3F));
        (*remaining) -= 3;
    }
    else if (codepoint <= 0x10FFFF) {
        // 4-byte sequence
        if (*remaining < 5) return false; // +1 for the null
        *(*d)++ = (char)(0xF0 | ((codepoint >> 18) & 0x07));
        *(*d)++ = (char)(0x80 | ((codepoint >> 12) & 0x3F));
        *(*d)++ = (char)(0x80 | ((codepoint >> 6) & 0x3F));
        *(*d)++ = (char)(0x80 | (codepoint & 0x3F));
        (*remaining) -= 4;
    }
    else
        // Invalid code point
        return false;

    return true;
}

size_t parse_utf8_surrogate(const char *s, char *d, size_t *remaining) {
    if (s[0] != '\\' || (s[1] != 'u' && s[1] != 'U')) {
        return 0; // Not a valid Unicode escape sequence
    }

    char hex[9] = {0}; // Buffer for the hexadecimal value
    unsigned codepoint;

    if (s[1] == 'u') {
        // Handle \uXXXX
        if (!isxdigit(s[2]) || !isxdigit(s[3]) || !isxdigit(s[4]) || !isxdigit(s[5])) {
            return 0; // Not a valid \uXXXX sequence
        }

        hex[0] = s[2];
        hex[1] = s[3];
        hex[2] = s[4];
        hex[3] = s[5];
        codepoint = (unsigned)strtoul(hex, NULL, 16);

        if (codepoint >= 0xD800 && codepoint <= 0xDBFF) {
            // Possible start of surrogate pair
            if (s[6] == '\\' && s[7] == 'u' && isxdigit(s[8]) && isxdigit(s[9]) &&
                isxdigit(s[10]) && isxdigit(s[11])) {
                // Valid low surrogate
                unsigned low_surrogate = strtoul(&s[8], NULL, 16);
                if (low_surrogate < 0xDC00 || low_surrogate > 0xDFFF) {
                    return 0; // Invalid low surrogate
                }
                codepoint = 0x10000 + ((codepoint - 0xD800) << 10) + (low_surrogate - 0xDC00);
                return encode_utf8(codepoint, &d, remaining) ? 12 : 0; // \uXXXX\uXXXX
            }
        }

        // Single \uXXXX
        return encode_utf8(codepoint, &d, remaining) ? 6 : 0;
    }
    else {
        // Handle \UXXXXXXXX
        for (int i = 2; i < 10; i++) {
            if (!isxdigit(s[i])) {
                return 0; // Not a valid \UXXXXXXXX sequence
            }
            hex[i - 2] = s[i];
        }
        codepoint = (unsigned)strtoul(hex, NULL, 16);
        return encode_utf8(codepoint, &d, remaining) ? 10 : 0; // \UXXXXXXXX
    }
}
