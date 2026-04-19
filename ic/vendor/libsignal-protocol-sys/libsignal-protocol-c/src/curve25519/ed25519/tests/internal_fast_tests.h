#ifndef __INTERNAL_FAST_TESTS_H__
#define __INTERNAL_FAST_TESTS_H__

/* silent = 0 : prints info+error messages to stdout, abort() on test failure
 * silent = 1 : returns 0 for success, anything else for failure 
 */

int sha512_fast_test(int silent);
int strict_fast_test(int silent);
int elligator_fast_test(int silent);
int curvesigs_fast_test(int silent);
int xeddsa_fast_test(int silent);
int vxeddsa_fast_test(int silent);
int generalized_xeddsa_fast_test(int silent);
int generalized_xveddsa_fast_test(int silent);

int all_fast_tests(int silent);

#endif
