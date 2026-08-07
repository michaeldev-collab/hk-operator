#include <array>
#include <cassert>
#include <cstring>

#include "cpad_base64.h"

int main() {
  std::array<uint8_t, 487> input{};
  for (size_t i = 0; i < input.size(); ++i) input[i] = uint8_t(i & 0xff);

  std::array<char, 700> encoded{};
  const int encoded_len =
      cpad_b64_encode(input.data(), input.size(), encoded.data(), encoded.size());
  assert(encoded_len == 652);
  assert(encoded[650] == '=' && encoded[651] == '=');

  std::array<uint8_t, 487> decoded{};
  assert(cpad_b64_decode(encoded.data(), size_t(encoded_len), decoded.data(),
                         decoded.size()) == int(decoded.size()));
  assert(decoded == input);

  assert(cpad_b64_decode("AA=A", 4, decoded.data(), decoded.size()) == -1);
  assert(cpad_b64_decode("AA==AAAA", 8, decoded.data(), decoded.size()) == -1);
  assert(cpad_b64_decode("AB==", 4, decoded.data(), decoded.size()) == -1);
  assert(cpad_b64_decode("AAB=", 4, decoded.data(), decoded.size()) == -1);
  assert(cpad_b64_decode("AA==", 4, decoded.data(), decoded.size()) == 1);
  return 0;
}
