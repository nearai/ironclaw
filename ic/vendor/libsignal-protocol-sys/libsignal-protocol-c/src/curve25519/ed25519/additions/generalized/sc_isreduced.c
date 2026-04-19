#include <string.h>
#include "fe.h"
#include "sc.h"
#include "crypto_additions.h"
#include "crypto_verify_32.h"

int sc_isreduced(const unsigned char* s)
{
  unsigned char strict[64];

  memset(strict, 0, 64);
  memmove(strict, s, 32);
  sc_reduce(strict);
  if (crypto_verify_32(strict, s) != 0)
    return 0;
  return 1;
}
