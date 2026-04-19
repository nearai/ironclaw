#ifndef SIGNAL_UTARRAY_H
#define SIGNAL_UTARRAY_H

#include "signal_protocol.h"

#define oom() do {                                                            \
  result = SG_ERR_NOMEM;                                                      \
  goto complete;                                                              \
} while(0)

#include "utarray.h"

#endif /* SIGNAL_UTARRAY_H */
