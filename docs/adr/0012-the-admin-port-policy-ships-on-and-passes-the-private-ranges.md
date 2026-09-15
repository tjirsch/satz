# 0012 — the admin-port firewall policy ships on, and passes the private ranges

- **Status:** accepted
- **Date:** 2026-09-12
- **Shipped in:** v0.52.0

## Context

`presets/cis-extensions/internet-ssh-rdp.satz` attaches a hierarchical firewall policy to
the organisation that denies TCP 22 and 3389 from the internet and passes listed ranges to
the VPC firewall rules. It satisfies CIS 4.0 / 5.0 §3.6 and §3.7.

It shipped opt-in, with its flag `cis_block_internet_ssh_rdp` defaulting to false, on the
reasoning that every SSH session reaching an instance from the internet ends the moment
the policy attaches. The consequence of an opt-in security control is the obvious one:
nothing in the fleet had it on. An organisation that has not closed the two
administrative ports to the internet is one scan away from the finding it will be asked
about, and this library exists to have answered that already.

Two facts decide the shape, and both come from Google's own documentation rather than
from the pack's description:

- **`0.0.0.0/0` is every IPv4 address, private ones included**, and a hierarchical policy
  is evaluated *before* the VPC firewall rules that would otherwise allow internal
  traffic. Google's own example puts an RFC1918 `goto_next` ahead of the `0.0.0.0/0` deny
  for exactly this reason. A pass list of only the IAP range therefore denies SSH between
  two instances in one subnet, and denies a bastion's onward hop as well.
- **A `goto_next` rule cannot log.** Google allows logging on `allow` and `deny` rules
  only. So whatever the pass rules do is unrecorded, and only refusals leave a firewall
  log.

## Considered options

1. **Keep it opt-in.** What we had.
2. **On by default, IAP range only.** The strictest reading of "deny SSH from the
   internet".
3. **On by default, IAP plus the private ranges** — deny the internet, hand internal
   traffic to the VPC rules.
4. **A custom org-policy constraint refusing permissive VPC rules** instead of a
   hierarchical policy.

## Decision

Option 3, with logging on the two deny rules.

The default pass lists are Google's IAP TCP-forwarding ranges (`35.235.240.0/20`, and
`2600:2d00:1:7::/64` for IPv6 VMs) plus `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
and `fc00::/7`. IPv4 and IPv6 are separate rules because a rule's sources may not mix the
two. Each rule carries the control's full protocol set — SSH on TCP 22 and SCTP 22, RDP on
TCP 3389 and UDP 3389 — because Google's detectors check all four and a TCP-only deny
leaves UDP 3389 open.

An estate that wants this off writes a `deviates` claim, whose `reason` is mandatory and
reaches the compliance report. An estate that wants internal SSH denied as well removes
the private blocks from the pass list.

## Consequences

- Every estate taking CIS pack 2.10 emits an organisation firewall policy it did not have.
  The plan shows it; the release is a MINOR by ADR 0010 because the plan moves.
- **A bastion reachable on a public address loses SSH when the policy attaches.** That is
  the control working, and the question says so before it is answered. The remedy is to
  list the bastion's range or move its access to IAP.
- Internal administration is untouched, which is what makes the default defensible. The
  cost is that the control is narrower than its strictest reading: an attacker already
  inside a VPC is not stopped from reaching port 22 by this policy. Nothing at the
  organisation's edge should be expected to stop that, and the estate can narrow it.
- An accepted IAP session leaves no firewall log, because the rule that admits it is a
  `goto_next`. Anyone looking for proof that IAP access works will not find it in the
  firewall logs; the denies are what the logs carry.
- The policy does not clear a Security Command Center `OPEN_SSH_PORT` finding, whose
  supported asset is the VPC firewall rule: the hierarchical deny shadows a permissive
  VPC rule without deleting it. `default-allow-ssh` and `default-allow-rdp` still have to
  go, which the pack's header says.

## Pros and cons of the options

### 1 · Keep it opt-in

- **Good:** no estate changes on upgrade; no risk of cutting access nobody expected.
- **Bad:** nobody turned it on. A control the library ships and no one runs is a control
  the library does not have.

### 2 · On by default, IAP only

- **Good:** the strictest reading, and the smallest pass list to audit.
- **Bad:** denies SSH between instances in one subnet, and a bastion's onward hop. Across
  a fleet that is an outage, not a hardening.

### 3 · On by default, IAP plus the private ranges *(chosen)*

- **Good:** denies what the control is about — the internet — and leaves internal
  administration to the VPC rules, where it belongs.
- **Good:** the pass list is explicit, so narrowing it is an edit rather than a fork.
- **Bad:** wider than the strictest reading; an estate that wants internal SSH denied has
  to say so.

### 4 · A custom constraint on VPC firewall rules

- **Good:** would clear the SCC finding, because it acts on the asset the detector reads.
- **Bad:** refuses rules at create time, so it breaks other people's pipelines, and it
  cannot reach the permissive rules that already exist. The hierarchical policy makes
  those rules ineffective immediately, which is the faster half of the job.
