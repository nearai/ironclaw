#include "crypto_additions.h"

/*
Replace (f,g) with (g,g) if b == 1;
replace (f,g) with (f,g) if b == 0.

Preconditions: b in {0,1}.
*/

void sc_cmov(unsigned char* f, const unsigned char* g, unsigned char b)
{
  int count=32;
  unsigned char x[32];
  for (count=0; count < 32; count++)
    x[count] = f[count] ^ g[count];
  b = -b;
  for (count=0; count < 32; count++)
    x[count] &= b;
  for (count=0; count < 32; count++)
    f[count] = f[count] ^ x[count];
}
