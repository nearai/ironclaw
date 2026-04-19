#include "crypto_additions.h"
#include "ge.h"

/*
return 8 * p
*/

void ge_scalarmult_cofactor(ge_p3 *q, const ge_p3 *p)
{
  ge_p1p1 p1p1;
  ge_p2 p2;

  ge_p3_dbl(&p1p1, p);
  ge_p1p1_to_p2(&p2, &p1p1);

  ge_p2_dbl(&p1p1, &p2);
  ge_p1p1_to_p2(&p2, &p1p1);

  ge_p2_dbl(&p1p1, &p2);
  ge_p1p1_to_p3(q, &p1p1);
}
