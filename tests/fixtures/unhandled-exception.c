/* Test-only PE fixture for verifying Macroton's Wine post-mortem debugger guard. */
int main(void)
{
    volatile int *invalid = (volatile int *)0;
    *invalid = 1;
    return 0;
}
