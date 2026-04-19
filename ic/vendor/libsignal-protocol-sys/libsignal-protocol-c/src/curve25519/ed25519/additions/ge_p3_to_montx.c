#include "fe.h"
#include "crypto_additions.h"

void ge_p3_to_montx(fe u, const ge_p3 *ed)
{
  /* 
     u = (y + 1) / (1 - y)
     or
     u = (y + z) / (z - y)

     NOTE: y=1 is converted to u=0 since fe_invert is mod-exp
  */

  fe y_plus_one, one_minus_y, inv_one_minus_y;

  fe_add(y_plus_one, ed->Y, ed->Z);
  fe_sub(one_minus_y, ed->Z, ed->Y);  
  fe_invert(inv_one_minus_y, one_minus_y);
  fe_mul(u, y_plus_one, inv_one_minus_y);
}

