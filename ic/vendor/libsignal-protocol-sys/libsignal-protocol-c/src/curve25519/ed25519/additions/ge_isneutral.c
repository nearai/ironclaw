#include "crypto_additions.h"
#include "ge.h"

/*
return 1 if p is the neutral point
return 0 otherwise
*/

int ge_isneutral(const ge_p3 *p)
{
  fe zero;
  fe_0(zero);

  /* Check if p == neutral element == (0, 1) */
  return (fe_isequal(p->X, zero) & fe_isequal(p->Y, p->Z));
}
