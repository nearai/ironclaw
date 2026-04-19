#include "fe.h"
#include "crypto_verify_32.h"

int fe_isreduced(const unsigned char* s)
{
  fe f;
  unsigned char strict[32];

  fe_frombytes(f, s);
  fe_tobytes(strict, f);
  if (crypto_verify_32(strict, s) != 0)
    return 0;
  return 1;
}
