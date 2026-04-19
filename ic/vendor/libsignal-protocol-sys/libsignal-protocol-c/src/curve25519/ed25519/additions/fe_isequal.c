#include "fe.h"
#include "crypto_verify_32.h"

/*
return 1 if f == g
return 0 if f != g
*/

int fe_isequal(const fe f, const fe g)
{
  fe h;
  fe_sub(h, f, g);
  return 1 ^ (1 & (fe_isnonzero(h) >> 8));
}
