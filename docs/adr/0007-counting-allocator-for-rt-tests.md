# 0007. Counting allocator instead of assert_no_alloc for RT tests

Date: 2026-11-13 · Status: accepted

## Context

§10.2 asks for `assert_no_alloc` around `process()` in tests. That crate aborts the process on
an allocation in debug builds, which kills the whole test binary, gives no count, and makes it
impossible to test the harness itself.

## Decision

`hearo_testkit::rt::CountingAllocator` is a global allocator that counts allocations,
deallocations and reallocations made on the current thread inside `count_allocs(|| ...)`.
RT tests assert the count is zero; a test proves the harness detects allocations. The same
intent as `assert_no_alloc`, with better diagnostics.

## Alternatives considered

- **assert_no_alloc** (1.1.2) — abort semantics; still a fine choice for a live debug build of
  the engine, and may be added there in M2.

## Consequences (incl. how we'd know it was wrong)

- Each RT test binary must declare `#[global_allocator]`. Wrong if: an allocation slips through
  on another thread we didn't guard (engine threads are covered in M2 tests).
