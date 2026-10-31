# Pluvora Kernel Design Policy

## 1. Purpose

This document defines the authoritative design policy for Pluvora and guides its implementation and detailed design decisions.

Pluvora does not attempt to eliminate every vulnerability. Its primary goal is to minimize the impact of a compromised component. The kernel therefore treats memory ownership, capability-based authority, and isolation as foundational principles and assumes that implementation defects may exist.

## 2. Core Principles

### 2.1 Memory Ownership

Every user-accessible memory region is represented by a Memory Object. The kernel explicitly tracks its owner, holders, and Read, Write, Execute, and Map permissions.

Authority is attached to a Memory Object rather than to a virtual address. Access is established by mapping a capability for that object into an Address Space.

### 2.2 Capability-Based Access Control

Access to kernel objects is granted only through explicit capabilities. Knowing an object identifier does not grant authority over that object.

A derived capability may retain or reduce the rights of its parent, but it may never amplify them.

### 2.3 Least Privilege

Every process, service, and driver receives only the capabilities required to perform its role.

### 2.4 Revocation

Capabilities are revocable. Revocation covers capability lookup, associated page-table mappings, relevant TLB entries on every affected hart, and in-flight kernel operations.

### 2.5 Isolation by Default

Every resource is inaccessible until an appropriate capability is explicitly granted.

## 3. Threat Model

### 3.1 Protection Scope

The initial implementation protects against:

- Malicious user processes
- Compromised user-space services

Pluvora primarily protects:

- Confidentiality
- Integrity

Compromised device drivers enter the protection scope after IOMMU and DMA isolation are implemented.

### 3.2 Out of Scope

The initial security model does not cover:

- Denial-of-service attacks
- Side-channel attacks
- Physical attacks
- Malicious hardware
- Faulty hardware

Finite queues and resource limits are still required to prevent accidental exhaustion of kernel resources.

### 3.3 Trusted Computing Base and Assumptions

The security mechanisms implemented by the S-mode kernel form the Trusted Computing Base.

The kernel assumes the correct operation of:

- The hardware
- The bootloader
- OpenSBI v1.9 using `fw_jump.bin`

The kernel never enters M-mode. OpenSBI owns the M-mode responsibilities.

## 4. Kernel Architecture

Pluvora follows a microkernel architecture. The kernel contains only the mechanisms required for isolation and resource management. System-level policy resides in user-space services wherever practical.

The kernel provides:

- Scheduling
- Memory management
- Capability management
- Inter-process communication
- Interrupt and trap handling
- Hardware abstraction

The following components run as user-space services:

- Filesystem servers
- Network stacks
- Device drivers
- Process managers
- Supervisor services
- Other system services

A user-space service is the unit of failure isolation and restart. An unrecoverable failure inside the kernel causes a kernel panic.

## 5. Kernel Object Model

### 5.1 Object Types

Capabilities govern the following object types:

- Memory Object
- Address Space
- Thread
- Process
- IPC Endpoint
- Interrupt
- MMIO Region
- Timer
- DMA Domain
- Device

The initial implementation covers every type above except complete Device Object management, which is introduced with user-space device drivers.

### 5.2 Creation and Destruction

Creating an object requires a Create capability for that object type. Destroying an object requires the Destroy capability held by its owner.

At boot, the root service receives the initial creation authority and delegates attenuated rights to other services.

### 5.3 Identity and Lifetime

Kernel objects reside in an internal Object Table and are referenced through slots in a Capability Space. Each reusable slot carries a generation number so that stale handles cannot refer to a newly allocated object.

Object lifetime accounts for capability references, page-table mappings, and temporary references held by in-flight kernel operations.

When an owner terminates, capabilities derived from that owner are revoked. The object is reclaimed only after it is unreachable from every hart.

## 6. Capability Model

### 6.1 Ownership and Possession

Object ownership and capability possession are distinct concepts.

An owner holds Destroy authority and comprehensive revocation authority. Other holders may perform only the operations encoded in their capabilities.

### 6.2 Representation and Validation

Every Process has an independent Capability Space. A user-visible capability handle consists of a slot number and a generation number. Object references and rights remain exclusively inside the kernel.

The kernel treats every user-supplied handle as untrusted and validates it against the caller's Capability Space for every operation.

### 6.3 Rights

Depending on the object type, a capability may contain these rights:

- Read
- Write
- Execute
- Map
- Share
- Transfer
- Grant
- Revoke
- Create
- Destroy

Grant permits a derived capability to be delegated to another Capability Space.

### 6.4 Derivation and Delegation

A derived capability may contain a strict subset of its parent's rights. Further delegation requires Grant authority.

Share creates a derived capability while retaining the source capability. Transfer moves a capability to another Capability Space and invalidates its source slot.

Copying, moving, or attaching a capability to an IPC message is atomic across the participating Capability Spaces and IPC Endpoint. A failed operation changes neither the message nor the capabilities.

## 7. Revocation Model

### 7.1 Scope

Normal revocation invalidates the selected capability and all of its descendants. Destroy invalidates every capability referencing the target object.

### 7.2 Completion

The initial implementation performs revocation synchronously. Revocation completes only when:

- New lookups through the affected capabilities fail
- Associated page-table mappings have been removed
- TLB entries have been invalidated on every hart that may execute the affected Address Space
- Temporary references acquired before revocation have been released
- Related in-flight IPC operations have completed or been cancelled

Once DMA isolation is implemented, completion also requires stopping relevant DMA operations and invalidating their IOMMU mappings.

### 7.3 Concurrency and Reuse

Object and capability locks, together with generation numbers, establish a linear order between capability use and revocation. New lookups fail after revocation begins, while earlier temporary references are drained.

A physical page may be reused only after every mapping has been removed, every affected hart has acknowledged its TLB shootdown, and all temporary references and DMA pins have disappeared.

Errors caused by a caller are reported to that caller. If the kernel cannot preserve a safe state, it quarantines the resource and panics when recovery is impossible.

## 8. Memory Model

### 8.1 Memory Objects

One Memory Object represents one page. The initial implementation does not split or merge Memory Objects. A higher-level collection represents a contiguous multi-page region.

The kernel Memory Manager allocates and reclaims physical memory.

### 8.2 Mapping

A Memory Object may be mapped into multiple Address Spaces only when the required Share or Map capability has been delegated explicitly.

Page-table permissions must never exceed the rights of the authorizing capability.

### 8.3 Copy-on-Write and W^X

Pluvora supports copy-on-write. A page created while resolving a copy-on-write fault receives ownership and capabilities distinct from the original Memory Object.

All user-space mappings enforce W^X. A mapping may not be writable and executable at the same time.

### 8.4 Data Sanitization

Every page newly exposed to user space is zero-filled. Immediate clearing at free time is not required, but clearing must occur before a page crosses into a different security domain.

Kernel memory is excluded from the user-space capability system and remains separate from user-accessible Memory Objects.

## 9. IPC and Service Isolation

### 9.1 Communication Model

IPC uses asynchronous message passing. Shared memory carries large payloads.

IPC Endpoints are capability-controlled and distinguish Send, Receive, Manage, and Grant rights.

### 9.2 Message Transfer

Small payloads are copied through fixed-size, kernel-managed messages. Page-sized and larger payloads use shared memory and Memory Capability transfer. The inline message limit is selected through implementation measurements.

Enqueuing a message and transferring its attached capabilities form one transaction. If either part fails, neither part takes effect.

### 9.3 Failure and Backpressure

An IPC Endpoint carries a generation number and liveness state. When its receiver terminates, blocked senders and the Supervisor Service receive a `PeerClosed` notification.

A restarted service receives new Endpoints and capabilities. Stale capabilities do not reconnect automatically.

Endpoint queues are bounded. On a full queue, a caller may request a non-blocking error or wait explicitly. Wait queues are priority-ordered, with priority donation applied where required.

## 10. Device and DMA Isolation

The initial implementation does not require an IOMMU. A compromised user-space driver that can initiate DMA is therefore outside the initial security guarantee.

Without an IOMMU, a DMA driver must either be trusted or use bounce buffers. Complete isolation from a compromised DMA-capable driver cannot be guaranteed in this configuration.

After IOMMU support is introduced, every device receives an independent DMA Domain. A Device Object acts as the parent of its MMIO Region, Interrupt, and DMA Domain capabilities. A Memory Object becomes a DMA Buffer when mapped into that DMA Domain.

Revoking a driver first blocks new DMA mappings, then stops or resets the device, waits for active DMA to finish, and finally removes IOMMU mappings and Memory Capabilities.

After a device reset, old capabilities and DMA mappings are not reused. New objects receive new generations.

## 11. RISC-V 64-bit Architecture

### 11.1 Execution Environment

The kernel runs in RISC-V S-mode and uses Sv39 virtual memory.

The initial target requires RV64IMA_Zicsr_Zifencei. The Atomic extension is mandatory for multi-hart synchronization. The Compressed extension is optional, and floating-point extensions are not required initially.

### 11.2 Page Tables

R/W/X bits in a Page Table Entry may not exceed the rights of the authorizing Memory Capability. The U bit is set only for user mappings. W and X are never enabled simultaneously.

### 11.3 ASIDs

Each Address Space receives an ASID. Before an ASID is reused, its generation changes and every hart invalidates the old ASID's TLB entries. If the available ASID space is exhausted, the kernel falls back to a global TLB flush.

### 11.4 PMP

OpenSBI and M-mode own PMP configuration. The S-mode kernel does not reconfigure PMP dynamically and manages only physical memory made available by OpenSBI.

### 11.5 Timers and Interrupts

Timers and IPIs use the interfaces provided by OpenSBI. External interrupts use the interrupt controller of the initial target platform.

An interrupt is routed to a selected hart and may be redistributed through an IPI when necessary.

## 12. Multi-Hart Model

Multi-hart operation is a fundamental requirement. No subsystem may rely solely on single-hart execution.

### 12.1 Hart Management

The boot hart initializes the kernel and starts the secondary harts. Every hart owns:

- A kernel stack
- Per-hart data
- A current Thread reference
- A scheduler run queue
- Interrupt state
- A TLB shootdown request queue

During a kernel panic, the initiating hart uses IPIs to stop all other harts.

### 12.2 Synchronization

Multi-hart synchronization uses the RISC-V Atomic extension. Short critical sections use spinlocks with acquire/release semantics. Operations that may wait for an extended period use sleeping locks.

Interrupt disabling and preemption disabling are separate mechanisms. The kernel defines a lock ordering, and the initial implementation avoids lock-free data structures.

### 12.3 TLB Shootdown

Each Address Space tracks the harts on which it is running or has run.

After changing a mapping, the initiating hart sends an IPI to each affected hart. Each target executes `SFENCE.VMA` for the Address Space and returns an acknowledgement. The page or ASID is not reused until every required acknowledgement has arrived.

### 12.4 Remote Operations

IPIs support remote Thread wakeups, interrupt redistribution, scheduler reevaluation, and TLB shootdowns.

Shared-data publication, page-table updates, and resource reclamation use memory ordering that is explicit and correct under the RISC-V memory model.

## 13. Scheduling and Resource Control

The scheduler is preemptive and round-robin, with a separate run queue for each hart.

Threads may carry CPU affinity. Load balancing uses periodic balancing or work stealing by idle harts.

The initial implementation does not provide hard real-time guarantees. It supports fixed priorities and uses priority donation for IPC and lock waits.

The kernel limits the following resources per Process or Service:

- Physical pages
- Capability slots
- Threads
- IPC queue capacity

Strict CPU-time quotas may be added later.

When a component terminates, its Threads, Address Space, Capability Space, and IPC Endpoints are stopped. Resources are reclaimed only after derived capabilities have been revoked safely.

## 14. Boot and Initial Authority

Secure Boot is outside the initial implementation. The bootloader, OpenSBI, and kernel image are assumed to have been loaded correctly.

After initializing the kernel and multi-hart environment, the boot hart creates the root service.

The root service receives the initial capabilities for memory, Process creation, devices, interrupts, and other user-managed resources. It never receives capabilities for kernel memory or internal kernel objects.

The kernel converts resources described by the Device Tree into objects and passes the corresponding capabilities to the root service.

The root service, or a Process Manager authorized by it, delegates authority to new Processes. The kernel verifies that no delegation amplifies authority.

## 15. Failure Handling and Recovery

The kernel detects Process termination and unhandled exceptions, then sends a Death Notification to a supervisory IPC Endpoint.

The user-space Supervisor Service owns restart policy. The kernel provides no service-specific restart policy.

A restarted service receives a new Process, IPC Endpoints, generations, and capabilities. Its previous capabilities are not restored directly.

Dependent services receive stop and restart notifications through supervisory IPC. Communication through an old Endpoint fails with `PeerClosed` or `StaleCapability`.

A Page Fault, illegal instruction, or invalid capability operation caused by a user process normally terminates that Process. An internal invariant violation, unrecoverable page-table inconsistency, or failed inter-hart synchronization causes a kernel panic.

## 16. Initial Implementation Order

The initial implementation prioritizes correct isolation and correct multi-hart behavior over features and performance.

The intended implementation order is:

1. Boot-hart and secondary-hart initialization
2. Per-hart data, kernel stacks, spinlocks, and IPIs
3. Physical memory management and Sv39 page tables
4. ASID management and inter-hart TLB shootdowns
5. Kernel Objects and Capability Spaces
6. Threads, Processes, and the multi-hart scheduler
7. Asynchronous IPC and capability transfer
8. Revocation and safe resource reclamation
9. The root service and Supervisor Service
10. User-space device drivers
11. IOMMU and DMA isolation

The first implementation favors synchronous, easily verifiable operations. Asynchronous revocation, deferred reclamation, and lock-free structures may be considered only after correctness has been established.
