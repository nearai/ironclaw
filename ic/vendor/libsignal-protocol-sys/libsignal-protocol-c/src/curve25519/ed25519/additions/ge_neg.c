#include "crypto_additions.h"
#include "ge.h"

/*
return r = -p
*/


void ge_neg(ge_p3* r, const ge_p3 *p)
{
  fe_neg(r->X, p->X);
  fe_copy(r->Y, p->Y);
  fe_copy(r->Z, p->Z);
  fe_neg(r->T, p->T);
}
