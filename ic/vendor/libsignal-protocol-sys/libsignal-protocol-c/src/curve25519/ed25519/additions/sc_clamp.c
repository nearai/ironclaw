#include "crypto_additions.h"

void sc_clamp(unsigned char* a)
{
  a[0] &= 248;
  a[31] &= 127;
  a[31] |= 64;
}
