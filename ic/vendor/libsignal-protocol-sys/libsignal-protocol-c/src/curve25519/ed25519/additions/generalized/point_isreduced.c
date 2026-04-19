#include<string.h>
#include "fe.h"
#include "crypto_additions.h"

int point_isreduced(const unsigned char* p)
{
  unsigned char strict[32];
 
  memmove(strict, p, 32);
  strict[31] &= 0x7F; /* mask off sign bit */
  return fe_isreduced(strict);
}
