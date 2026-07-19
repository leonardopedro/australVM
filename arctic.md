# Arctic: Lightweight and Stateless Threshold Schnorr Signatures

Chelsea Komlo Ian Goldberg

$^{1}$ University of Waterloo &amp; NEAR One
$^{2}$ University of Waterloo
ckomlo@uwaterloo.ca, iang@uwaterloo.ca

Abstract. Threshold Schnorr signatures are seeing increased adoption in practice, and offer practical defenses against single points of failure. However, one challenge with existing randomized threshold Schnorr signature schemes is that signers must carefully maintain secret state across signing rounds, while also ensuring that state is deleted after a signing session is completed. Failure to do so will result in a fatal key-recovery attack by re-use of nonces.

While deterministic threshold Schnorr signatures that mitigate this issue exist in the literature, all prior schemes incur high complexity and performance overhead in comparison to their randomized equivalents. In this work, we seek the best of both worlds; a deterministic and stateless threshold Schnorr signature scheme that is also simple and efficient.

Towards this goal, we present Arctic, a lightweight two-round threshold Schnorr signature that is deterministic, and therefore does not require participants to maintain state between signing rounds. As a building block, we formalize the notion of a Verifiable Pseudorandom Secret Sharing (VPSS) scheme, and define $\mathsf{VPSS}_1$, an efficient VPSS construction. $\mathsf{VPSS}_1$ is secure when the total number of participants is at least $2t - 1$ and the adversary is assumed to corrupt at most $t - 1$; i.e., in the honest majority model.

We prove that Arctic is secure under the discrete logarithm assumption in the random oracle model, similarly assuming at minimum $2t - 1$ number of signers and a corruption threshold of at most $t - 1$. For moderately sized groups (i.e., when $n \leq 20$), Arctic is more than an order of magnitude more efficient than prior deterministic threshold Schnorr signatures in the literature. For small groups where $n \leq 10$, Arctic is three orders of magnitude more efficient.

# 1 Introduction

Threshold signature schemes allow a subset of at least $\mu$ out of $n$ possible parties to cooperate to produce a signature over a single message, while preserving security when up to $t - 1$ signers may be corrupted, where $\mu \leq n$ and $t \leq \mu$. Threshold signatures offer practical security benefits, allowing for dynamic key

management and defense in depth against potential adversarial corruptions. While threshold signatures can be instantiated by simply concatenating multiple individual signatures into a single joint signature, a practical goal for threshold signatures is to preserve compatibility with existing single-party signature schemes, to minimize implementation complexity.

In this work, we focus our attention on threshold Schnorr signatures; i.e, a threshold signature scheme that is compatible with the (single-party) Schnorr signature verification algorithm *[52]*. Efficient (two-round) threshold Schnorr signatures exist in the randomized setting *[36, 1, 51]*, where each party is assumed to securely maintain state between signing rounds and have access to good sources of randomness. However, efficient and *deterministic* threshold Schnorr signatures has thus far remained an elusive goal.

#### Why Deterministic Threshold Signatures?

Producing threshold signatures in a deterministic manner is useful for two reasons. First, it is useful as a general defense-in-depth measure, to protect against the event of temporarily losing access to good sources of randomness *[50]*, such as if a machine randomly rebooted.

Second, threshold Schnorr signature schemes generally require participants to perform multiple rounds of communication before a joint signature can be issued. As such, participants must keep state between each round, and carefully delete state once a signing protocol is completed. In the setting where the signer must produce signatures at increasing scale and in a concurrent setting, managing state can become a significant performance bottleneck. Further, secure management of secret state can be considered a security risk. In particular, for threshold Schnorr signatures, participants must generate secret nonces for each signing session. If a participant’s state is mismanaged in such a way that it is used even twice, a fatal key-recovery attack is possible. However, in the setting where machines can go offline at any time, and signing is done at scale and concurrently, such careful management of secret state becomes even more challenging.

#### The Challenge of Efficient Deterministic Multi-Party Schnorr Signatures.

While the goal of efficient and deterministic multi-party Schnorr signature schemes is desirable, producing such schemes has proven difficult. The challenge can be summarized as follows. While honest participants can certainly pick their nonces deterministically (say, by hashing their secret signing share and the message), a malicious party might deviate from the protocol and pick its nonce at random. Recall that for Schnorr signatures, the output signature $\sigma=(R,z)$ is a commitment $R$ and response $z$, satisfying the relation $g^{z}=R\cdot\mathsf{pk}^{c}$ for a challenge $c\leftarrow H(R,\mathsf{pk},m)$. In the setting for threshold Schnorr signatures, where $(R,z)$ are contributed to by all signing parties, one party deviating from the protocol results in the challenge $c$ likewise changing. If honest parties cannot verify that other parties followed the protocol, such a deviation would allow an adversary to perform a key recovery attack with as little as two signing queries.

Prior deterministic multi-party Schnorr signature schemes in the literature *[25, 39, 46]* can be broken down into two general approaches. We give an overview of

the approaches that support the general threshold setting with general $(t,\mu,n)$ in Table 1, with more context now.

The first approach requires that signers output zero-knowledge proofs that a Pseudorandom Function (PRF) was evaluated correctly *[25, 46]*. However, this approach adds undesirable performance and complexity overhead. For example, while MuSig-DN *[46]* requires only two network rounds, proof generation requires approximately 1 second (regardless of the number of signers) for 256-bit security, and verification requires at least $10n\text{\,}\mathrm{ms}$. Similarly, the scheme by *Garillot et al. [25]* requires three rounds of communication between signers and high complexity and bandwidth overhead.

The second approach by *Kondi et al. [39]* is to use as a black box a Pseudorandom Correlation Generator (PCG) *[9]* to generate nonces in a verifiable manner *[39]*. However, while the PCG used in their construction can support 2-of-2 signing, *[39, 47]*, it remains an open problem as to how to extend such a PCG to the general threshold setting for any $t$, $\mu$, or $n$ *[38]*. As such, the scheme by *Kondi et al. [39]* supports only the $t=\mu=n=2$ setting.

Therefore in this work, we address the following question:

> Can we design a simple, deterministic, and stateless two-round threshold Schnorr signature scheme, with similar efficiency to existing randomized schemes, and acceptable security assumptions in practice?

### 1.1 Our Results

We answer the above question in the affirmative. In particular, we present Arctic, a deterministic and stateless threshold Schnorr signature scheme. For moderately sized groups (i.e., when $n\leq 20$), Arctic is more than an order of magnitude more efficient than existing deterministic schemes in the literature; it is three orders of magnitude more efficient when $n\leq 10$. For even larger groups, Arctic scales linearly relative to the number of processing cores available to most machines. Signers in Arctic are required to generate and maintain secret keys, but otherwise do not need to manage secret state.

To achieve efficient determinism, Arctic requires three tradeoffs, as follows.

#### Tradeoff One: Honest Majority Assumption.

The first tradeoff is in the number of required signers; Arctic requires $\mu\geq 2t-1$ parties to participate in signing. This requirement is because Arctic assumes the honest majority model. We discuss in Section 1.2 why the assumption of an honest majority is acceptable for some real-world applications.

#### Tradeoff Two: Performance Scales Relative to Number of Participants.

The second tradeoff is in the required overhead as the signing set grows large. Under the hood, Arctic employs a replicated secret sharing scheme *[32]*, and so requires participants to store a secret key of size $\binom{n-1}{t-1}$. As such, Arctic outperforms other schemes in the literature for moderately sized groups, but incurs a crossover point as the signing set grows large.

|   | Computation | Bandwidth | Num. Rhymes | Assumptions | Min. Signers  |
| --- | --- | --- | --- | --- | --- |
|  MuSig-DN [46] | 14210 + 24(n-1) group | 1189 bytes | 2 | DDH | n  |
|  GKMN21 [25] | 132,000(t-1) AES 256(t-1) field | 1.01(t-1) MB | 3 | PRF | t  |
|  Arctic (this work) | 2t2-t+2 group 2(n-1t-1) field, 2(n-1t-1) hash | 97 bytes | 2 | DL | 2t-1  |

Table 1: Efficiency comparison of multi-party deterministic Schnorr signature schemes in the random oracle model (ROM) that support general choices of  $n$  (i.e., we exclude 2-of-2 schemes). Here,  $n$  denotes the number of total possible signers,  $t$  denotes the corruption threshold. Computation denotes the computational overhead per signing participant, and is given with respect to the operations that dominate; estimates for MuSig-DN are given with respect to a 256-bit elliptic curve. More information is given in the full version of this work [37, App.A]. Bandwidth denotes the bandwidth sent by each signing participant, and similarly is given with respect to a 256-bit elliptic curve [25, 46].

Tradeoff Three: No Identifiable Abort. The final tradeoff is that Arctic as defined does not support identifiable abort. While the protocol allows honest participants to identify that misbehavior has occurred, the protocol does not support identifying which participant caused the abort (which is possible for MuSig-DN [46], for example). However, we give a discussion on how Arctic can be extended to support identifiable abort, and indeed robustness, in the honest supermajority setting (i.e., when  $\mu \geq 3t - 2$ ).

Verifiable Pseudorandom Secret Sharing (VPSS). As a building block, we first formalize a cryptographic primitive that we call a Verifiable Pseudorandom Secret Sharing (VPSS) scheme. While pseudorandom secret sharing is a standard notion in the literature, and verifiability of such schemes had been employed implicitly in maliciously secure MPC-based schemes [4], we present a formal definition for a VPSS and give game-based security notions [3], building on existing notions of verifiable random functions in the literature [19, 24, 43, 44].

Intuitively, a pseudorandom secret sharing scheme allows a coalition of players holding pre-established secret shares of a random secret to generate shares of additional pseudorandom secrets. A verifiable pseudorandom secret sharing scheme is simply an extension to allow for public verifiability by collectively verifying the outputs for the set of participants. As such, misbehavior of players in a VPSS can be detected assuming some threshold of honest participants. As

we will see in the case of Arctic, a VPSS allows for more efficiently verifying the correctness of the output of a pseudorandom function that is distributed among a set of players, without requiring each player individually to produce a zero-knowledge proof that it followed the protocol.

We then define a concrete VPSS construction that we call $\mathsf{VPSS}_{1}$ that builds upon the pseudorandom secret sharing scheme by Cramer et al. *[15]*. We augment the scheme by Cramer et al. to additionally define a verification algorithm that is publicly verifiable even when players only publish commitments to their shares; i.e., it preserves secrecy of players’ shares while allowing players to verify that all other players followed the protocol honestly. $\mathsf{VPSS}_{1}$ is efficient for moderately sized groups; concretely, evaluation requires $\binom{n-1}{t-1}$ sub-microsecond hash and field operations, and verification requires $2t^{2}-t$ group exponentiations, where $t$ is the corruption threshold. We prove that $\mathsf{VPSS}_{1}$ is secure assuming the existence of a secure hash function in the honest majority model, i,e., when $\mu\geq 2t-1$, where $\mu$ is the minimum number of participants required to participate in the protocol. Additionally, we require that as $n$ grows, $t$ remains $\mathcal{O}(1)$ so that $\binom{n-1}{t-1}=\text{poly}(n)$.

#### A New Two-Round, Deterministic, and Stateless Threshold Schnorr Signature.

Next, we introduce Arctic, a new two-round, deterministic, and stateless threshold Schnorr signature. Arctic is an order of magnitude more efficient than related schemes in the literature for moderately sized groups ($n\leq 20$), due to its use of $\mathsf{VPSS}_{1}$ as a building block. Furthermore, Arctic supports the general threshold setting for any $t,\mu$ and $n$, so long as $\mu\geq 2t-1$ and $\mu\leq n$. We give an overview of Arctic in Table 1 in comparison to related schemes in the literature.

At a high level, Arctic uses $\mathsf{VPSS}_{1}$ in the first round of signing to generate participant nonces and commitments; participants publish their commitments without having to maintain state of their (secret) nonce. Then, in the second round, participants use $\mathsf{VPSS}_{1}$ to re-derive their nonce and commitment, and verify all other participants’ commitments, ensuring that other participants followed the protocol correctly. If the verification check holds, participants derive the group commitment as the aggregation of all participants’ individual commitments, and generate a signature share as a combination of their nonce, challenge, and secret signing share. The output signature is the aggregated group commitment and an aggregation of all participants’ signature shares, and can be verified using the single-party Schnorr verification algorithm.,

We prove the unforgeability of Arctic assuming the hardness of the discrete logarithm problem in the random oracle model, assuming $\mu\geq 2t-1$, the adversary corrupts at most $t-1$ players, and $\binom{n-1}{t-1}=\text{poly}(n)$. We describe in Section 1.2 why requiring honest majority assumptions can be practical for some real-world deployments of threshold signatures, at the benefit of improved simplicity and performance.

#### Performance Analysis of Arctic.

To estimate the practicality of Arctic for various choices of $n,t$, and $\mu$, we implement the scheme and provide concrete benchmarks.

Arctic is highly efficient for moderately sized groups, and so is a good choice for such settings. As we show in greater detail in Section 6, for a group where $2t-1\leq\mu\leq n\leq 20$, Arctic signing operations require less than 100 milliseconds of computation for each signer, and for $n\leq 10$, less than 1 millisecond. Signing however increases in cost relative to the corruption threshold; for signing sets of size $n=25,t=11$, the computational overhead for each signer on a single core requires one second. However, Arctic is highly parallelizable, showing almost perfect linear speedups for up to 32 cores for that size of signing set.

For comparison, MuSig-DN requires about 1 second in computational overhead per signer *[46]*, regardless of the size of the signing set. As such, Arctic is a practical choice for moderately sized groups of $n\leq 25$, whereas MuSig-DN or GKMN21 *[25]* may be good choices as the set of signers grows large, depending on the parallelization of each scheme.

#### Our Contributions.

In summary, the contributions of our work are as follows:

- We formalize the definition of a Verifiable Pseudorandom Secret Sharing (VPSS) scheme, and give game-based security notions.
- We define a concrete VPSS scheme that we call $\mathsf{VPSS}_{1}$, which extends the pseudorandom scheme by Cramer et al. *[15]* to allow for public verifiability, assuming an honest majority of participants.
- We then present Arctic, a deterministic and stateless two-round threshold Schnorr signature scheme. Arctic uses $\mathsf{VPSS}_{1}$ as a building block to generate participant nonces and commitments, and verify that participants performed this action correctly.
- We prove that Arctic is secure under the discrete logarithm problem in the random oracle model, assuming $\mathsf{VPSS}_{1}$ is a secure VPSS, the adversary corrupts fewer than $t$ parties, at least $\mu\geq 2t-1$ parties participate in the signing protocol, and $\binom{n-1}{t-1}=\text{poly}(n)$.
- We provide performance benchmarks for Arctic for different sizes of signing sets, and show that parallelization enables a linear speedup in signer computation.

### 1.2 Observations of Honest Majority Assumptions in Practice

While honest minority assumptions may be desirable for some real-world applications, we observe that honest majority assumptions may be an acceptable tradeoff for other applications in exchange for statelessness, improved performance, and protocol simplicity.

For applications that can easily support additional signers, moving from an honest minority setting to honest majority can be relatively straightforward. For example, an application that currently uses a $(t=2,\mu=2,n=3)$ configuration can instead move to a $(t=2,\mu=3,n=4)$ configuration.

In settings that require liveness, honest majority requirements are already assumed, to ensure usability of the shared secret key even when corrupt players refuse to participate. For example, applications such as cryptocurrency wallets

often implicitly require honest majority assumptions, to ensure that corrupt players cannot prevent use of funds by simply refusing to participate in signing operations.

Finally, some applications may see the requirement for additional signers as an acceptable tradeoff to mitigate the security risk of protocol complexity, as protocol complexity increases the risk of exploitable implementation errors *[42]*.

## 2 Related Work

#### Randomized (Non-Deterministic) Threshold Schnorr Signatures

We review only threshold Schnorr signatures in the literature that are secure in a concurrent setting and therefore demonstrated to be secure against ROS attacks *[5, 21, 53]*.

Stinson and Strobl *[55]* present a five-round threshold Schnorr signature that uses a robust DKG *[29]* for generating its nonce. However, the protocol assumes all participants choose their inputs in a randomized manner.

FROST *[14, 36]* is a randomized two-round threshold Schnorr signature that is secure even when the first round is preprocessed; i.e, it is performed in a batched manner resulting in only a single online signing round. FROST is secure assuming the One-More Discrete Logarithm (OMDL) assumption in the Random Oracle Model (ROM) *[1]*. Variants of FROST have been presented to improve its computational and bandwidth efficiency, including FROST2 *[1]* and FROST3 *[13, 51]*. Wong et al. *[56]* propose a (randomized) extension of FROST to derive nonces by hashing randomness with additional deterministic factors. However, FROST and these variants rely on participants selecting their nonces at random.

Thee-round randomized threshold Schnorr signatures have likewise been proposed. Lindell *[40]* presents a three-round threshold Schnorr signature scheme that relies on Fischlin zero-knowledge proofs *[23]* to demonstrate its simulatability with respect to Schnorr under aborts. Sparkle *[16]* is a three-round threshold Schnorr signature that is secure assuming the Discrete Logarithm (DL) assumption in the ROM. Sparkle does not rely on heavyweight proofs of knowledge, and instead demonstrates unforgeability via a game-based definition. Makriyannis *[41]* similarly presents two separate three-round threshold Schnorr signatures, which achieve a similar notion of security as to Sparkle. However, each scheme likewise relies upon randomized nonces.

#### Deterministic Threshold Signatures

The BLS signature scheme *[7]* is deterministic, because the signature is of the form $z\leftarrow H(m)^{\text{sk}}$. Likewise, threshold BLS signatures are similarly deterministic *[6]*. However, such schemes are often not viable in a practical setting that requires backwards compatibility with Schnorr signature verification, or due to the requirement of pairings.

Deterministic threshold Schnorr signatures have been described in the literature, but rely on heavyweight zero-knowledge proofs to demonstrate that each participant followed the protocol correctly. The challenge of deterministic

threshold Schnorr signatures is that of *verifiability*, because if even one participant deviates from the protocol and chooses its nonce randomly, a complete key-recovery attack is possible.

Nick et al. *[46]* define a threshold Schnorr $n$-of-$n$ multisignature that uses SNARKs to demonstrate in zero-knowledge that a participant generated its nonce using a PRF correctly with respect to a pre-established keypair. However, the authors report the computational overhead of at least $943\text{\,}\mathrm{ms}$ for a single execution, independent of the number of signers, due to the overhead of proving the PRF was evaluated correctly in zero-knowledge. In particular, to instantiate the PRF, a non-algebraic cryptographic hash function $H:\{0,1\}\to\mathbb{G}$ must be used, along with a regular function $f:\mathbb{G}\to\mathbb{Z}_{q}$. The complete PRF $F_{\mathsf{sk}}$ is defined by $F_{\mathsf{sk}}(x)=f(\mathsf{sk}\cdot H(x))$, where $\mathsf{sk}$ is a PRF key. Then, signers generate a Bulletproof *[10]* to prove in zero-knowledge that the nonce was derived with respect to $F$, $\mathsf{sk}$, and some input $H(x)$.

Bronte et al. *[8]* define an MPC protocol to partition the functionality of EdDSA, which itself is deterministic. However, their MPC protocol is randomized and requires multiple rounds of interaction, and therefore is stateful.

Garillot et al. *[25]* similarly rely on parties sampling and committing to a PRF key at the time of key generation, but instead make use of the Zero-Knowledge from Garbled Circuits (ZKGC) paradigm *[34]* to demonstrate that the PRF was derived correctly. The specific function that is garbled is $C(x)=\phi(F(x))$, where $F$ is a boolean circuit such as AES, and $\phi$ is standard exponentiation (or curve multiplication). The authors give efficiency estimates for a $256$-bit curve and using SHA-512 as the PRF $F$; the performance overhead is dominated by performing $132,000$ AES invocations and $256$ additions in $\mathbb{Z}_{q}$ per proof generated and verified. For a signing invocation involving $t$ parties, each party must then perform $256t$ field operations and $132,000t$ AES invocations, accounting for each proof a signer generates and the $t-1$ proof verifications for all other signers.

Kondi, Orlandi, and Roy *[39]* take a different approach, and define a two-round stateless and deterministic two-party Schnorr signature scheme using pseudo-random correlation functions (PCFs) *[9]* as a building block. In particular, their scheme employs a Paillier-based PRF *[47]*, but additionally define a verification mechanism to ensure that parties honestly followed the protocol. However, their scheme is restricted to the two-party setting, and as written, cannot be extended to the general $(t,n)$ threshold setting. In particular, the PCF used as a building block by their scheme assumes only two parties. Extending their scheme to any $(t,n)$ setting requires designing a new $n$-party PCF *[38]*.

##### Honest Majority Threshold Signatures.

Honest majority threshold schemes have been proposed in the literature as a means to achieve properties that are either impossible, or require higher performance overhead, in the honest minority setting. Notably, honest majority schemes have been demonstrated to achieve robustness or to circumvent requiring the use of heavyweight zero-knowledge proofs *[11]*.

Gennaro et al. *[27]* use error correcting codes in the honest majority setting to achieve a robust threshold DSS scheme. Similarly, Ruffing et al. *[51]* present a

wrapper to the FROST threshold signature scheme to achieve robustness, in the honest majority setting.

In the randomized threshold ECDSA setting, *Damgård et al. [18], Doerner et al. [20]*, and *Delskov [17]* show how the pseudorandom secret sharing scheme by *Cramer et al. [15]* can be employed as a sub-protocol for improved efficiency. However, our work targets the setting of deterministic threshold Schnorr signatures.

##### Distributed Randomness

*Galindo et al. [24]* give a formalization of distributed VRFs and their security notions. While our notions for a Verifiable Pseudorandom Secret Sharing (VPSS) scheme are similar, our definition for a VPSS does not require that players’ outputs are accompanied by a zero-knowledge proof that the protocol was performed correctly. Instead, the VPSS verification function *collectively* verifies all parties’ outputs.

*Cascudo and David [12]* define an honest-majority random beacon with optimized verification techniques under the decisional Diffie-Hellman (DDH) assumption, improving the cost of verification from $\mathbb{O}(n\cdot t)$ exponentiations to $\mathbb{O}(n)$. However, their PVSS verifies the consistency of $n$ shares with respect to the committed secret, but does not verify that the resulting polynomial is of degree $t-1$, and so their efficiency improvements cannot be employed in the context of $\mathsf{VPSS}_{1}$.

##### Pseudorandom Secret Sharing

Our VPSS construction $\mathsf{VPSS}_{1}$ builds on the pseudorandom secret sharing scheme by *Cramer et al. [15]*. Note that while Cramer et al. additionally define a Non-Interactive Verifiable Secret-Sharing (NIVSS) variant, their construction assumes that players output shares in the clear, and requires an honest supermajority so that the secret can be recovered. In our case, players verify the correctness of their shares with respect to other players’ public commitments, and requires only honest majority (as players simply need to determine if any other party deviated from the protocol).

While we are the first to do so in a threshold Schnorr signature setting, pseudorandom secret sharing has been used as a building block in other other threshold settings. For example, *Jarecki et al. [33]* define a threshold Oblivious PRF which likewise builds upon pseudorandom secret sharing.

##### Concurrent Work

Concurrently to this work, *Katz [35]* gives several distributed key generation (DKG) protocols, one of which likewise builds upon the pseudorandom secret sharing scheme by *Cramer et al. [15]*. However, our work shows how this technique can be securely employed in the context of threshold Schnorr signatures.

## 3 Preliminaries

### 3.1 General Notation

Let $\lambda\in\mathbb{N}$ be a security parameter. We denote the assignment of an element $y$ to the value $x$ as $y\leftarrow x$, and sampling an element from some set $S$ uniformly at

random as $x \xleftarrow{\iota} S$. For a randomized algorithm $A$, we write $x \xleftarrow{\iota} A()$ to indicate the random variable $x$ that is output from the execution of $A$.

We use $[n]$ to represent the set $\{1, \dots, n\}$. For a set $S$, we denote $\binom{S}{t}$ to mean the set that consists of all size-$t$ subsets of $S$.

**Groups and Group Generation.** Let $\mathbb{G}$ be a cyclic group of prime order $q$, and $\mathbb{Z}_q$ be the field of integers modulo $q$. Let $g$ be a generator of $\mathbb{G}$, and let $I_{\mathbb{G}} \in \mathbb{G}$ be the identity element of $\mathbb{G}$.

We use $\mathsf{GroupGen}(1^{\lambda})$ to denote a polynomial-time algorithm that takes as input a security parameter $\lambda$ and outputs a group description $(\mathbb{G}, q, g)$.

**Polynomial Interpolation.** A polynomial of degree $t - 1$ over a field $\mathbb{F}$ can be interpolated by $t$ (or more) points. Let $\eta$ be the list of $t$ distinct indices $\eta \subseteq [n]$ corresponding to the x-coordinates $x_i \in \mathbb{F}, i \in \eta$. Then the $L_i(x)$ (for $i \in \eta$) are the Lagrange polynomials defined by $\eta$, of the form $L_i(x) = \prod_{j \in \eta; j \neq i} \frac{x - x_j}{x_i - x_j}$. Later in this work, we denote $L_i(0)$ as $\lambda_i$.

Given a set of $t$ points $(x_i, f(x_i))$, any point $f(x_\ell)$ on the degree $t - 1$ polynomial $f$ can be determined by Lagrange interpolation: $f(x_\ell) = \sum_{j \in \eta} f(x_j) \cdot L_j(x_\ell)$.

## 3.2 Definitions and Assumptions

**Assumption 1 (Discrete Logarithm Assumption (DL))** The discrete logarithm assumption holds for GroupGen if for all PPT adversaries $\mathcal{A}$, $\mathsf{Adv}_{\mathcal{A}}^{\mathrm{dl}}(\lambda)$ is negligible, where

$$
\mathsf{Adv}_{\mathcal{A}}^{\mathrm{dl}}(\lambda) = \Pr \left[ \begin{array}{l}
(\mathbb{G}, q, g) \leftarrow \mathsf{GroupGen}(1^{\lambda}) \\
X \xleftarrow{\iota} \mathbb{G} \\
x' \xleftarrow{\iota} \mathcal{A}((\mathbb{G}, q, g), X)
\end{array} : \quad X \stackrel{?}{=} g^{x'} \right]
$$

**Schnorr Signatures.** A Schnorr signature is a Sigma protocol zero-knowledge proof of knowledge of the discrete logarithm of the public key, made non-interactive and bound to the message $m$ by the Fiat-Shamir transform [22]. Schnorr signatures are secure under the discrete logarithm assumption in the random oracle model [49].

**Definition 1 (Schnorr Signatures [52]).** The Schnorr signature scheme is defined as follows:

- **Schnorr.Setup** $(1^{\lambda}) \to \mathsf{par}$: On input the security parameter, run $(\mathbb{G}, q, g) \gets \mathsf{GroupGen}(1^{\lambda})$ and select a hash function $\mathsf{H} : \{0, 1\}^* \to \mathbb{Z}_p$. Output public parameters $\mathsf{par} = ((\mathbb{G}, q, g), \mathsf{H})$ (which are given implicitly as input to all other algorithms).

- Schnorr.KeyGen() $\rightarrow(\mathsf{pk},\mathsf{sk})$: Sample a secret key $\mathsf{sk}\xleftarrow{\iota}\mathbb{Z}_{q}$ and compute a public key $\mathsf{pk}\leftarrow g^{\mathsf{sk}}$. Output $(\mathsf{pk},\mathsf{sk})$.
- Schnorr.Sign($\mathsf{sk},m)\rightarrow\sigma$: On input secret key $\mathsf{sk}$ and message $\mathsf{m}$, the signer samples a nonce $r\xleftarrow{\iota}\mathbb{Z}_{q}$ and computes a nonce commitment $R\leftarrow g^{r}$. The signer then computes the challenge $c\leftarrow\mathsf{H}(R,\mathsf{pk},\mathsf{m})$ and the response $z\leftarrow r+cx$. Output the signature $\sigma=(R,z)$.
- Schnorr.Verify($\mathsf{pk},\mathsf{m},\sigma)\rightarrow 0/1$ : On input the public key $\mathsf{pk}$, a message $\mathsf{m}$, and a purported signature $\sigma=(R,z)$, the verifier computes $c\leftarrow\mathsf{H}(R,\mathsf{pk},\mathsf{m})$ and accepts if $g^{z}=R\cdot\mathsf{pk}^{c}$.

Shamir secret sharing. The $(t,n)$ Shamir secret sharing scheme *[54]* allows a dealer to partition a secret into $n$ shares, $t$ of which are required to recover the secret. Shamir secret sharing is information-theoretically secure.

###### Definition 2 (Shamir secret sharing *[54]*).

Shamir secret sharing Shamir is a threshold secret sharing scheme that consists of the following algorithms:

- Shamir.Share($s,n,t)\rightarrow\{(1,\phi_{1}),\ldots,(n,\phi_{n})\}$: On input a secret $s$, the number of participants $n$, and a threshold $t$, perform the following. First, define a polynomial $f(x)=s+a_{1}+a_{2}x^{2}+\cdots+a_{t-1}x^{t-1}$ by sampling $t-1$ coefficients at random $(a_{1},\ldots,a_{t-1})\xleftarrow{\iota}\mathbb{Z}_{q}$. Then, set each participant’s share $\phi_{i},i\in[n]$, to be the evaluation of $f(i)$: $\phi_{i}\leftarrow s+\sum_{j\in[t-1]}a_{j}i^{j}$. Output $\{(i,\phi_{i})\}_{i\in[n]}$.
- Shamir.Recover($t,\{(i,\phi_{i})\}_{i\in\eta})\rightarrow\bot/\mathsf{sk}$: On input a threshold $t$ and a set of shares $\{(i,\phi_{i})\}_{i\in\eta}$, output $\bot$ if $\eta\not\subseteq[n]$ or if $|\eta|<t$. Otherwise, compute $L(x)=\sum_{i\in\eta}w_{i}L_{i}(x)=\sum_{i\in\eta}w_{i}\prod_{j\in\eta,j\neq i}\frac{x-j}{i-j}$. If $\deg(L_{i})>t-1$, return $\bot$. Otherwise, return $s=L(0)=\sum_{i\in\eta}w_{i}L_{i}(0)=\sum_{i\in\eta}w_{i}\prod_{j\in\eta,j\neq i}\frac{j}{j-i}$.

### 3.3 General Forking Lemma

We next review the general forking lemma by Bellare and Neven *[2]*, which itself is a formalization of the forking lemma introduced by Pointcheval and Stern *[48]*. We show the general forking algorithm in Figure 1.

###### Lemma 1 (General Forking Lemma *[2]*).

Let $H$ be a finite set and $r\geq 1$ be an integer. Let $\mathsf{IG}$ be a randomized instance generator and let $X\xleftarrow{\iota}\mathsf{IG}$ be an instance. Let $\mathcal{C}$ be a randomized algorithm that takes as input $X$, quantities $h_{1},\ldots,h_{r}\in H$, and a random tape $\rho$. Let $\mathsf{accept}(\mathcal{C})$ be the probability that $\mathcal{C}$ outputs an accepting answer, namely

\[ \mathsf{accept}(\mathcal{C}):=\Pr\left[j\neq\bot\ :\begin{array}[]{c}X\xleftarrow{\iota}\mathsf{IG},\ \ h_{1},\ldots,h_{r}\xleftarrow{\iota}H\\
(j,\mathsf{aux})\xleftarrow{\iota}\mathcal{C}\big{(}X,(h_{1},\ldots,h_{r});\rho\big{)}\end{array}\right]. \]

######

![img-0.jpeg](img-0.jpeg)
Fig. 1: The general forking algorithm  $\mathsf{Fork}^{\mathcal{C}}(X)$  and the modified general forking algorithm  $\mathsf{Fork}_m^{\mathcal{C}}(X)$ , defined with respect to an algorithm  $\mathcal{C}$  and instance  $X$ . The difference between the general forking algorithm and modified variant is shown in a box, for emphasis. In summary, the modified general forking algorithm aborts if any of the  $(h_1,\ldots ,h_r,h_j',\ldots ,h_r')$  collide.

Let  $\mathsf{Fork}^{\mathcal{C}}(X)$  be the general forking algorithm defined in Figure 1 and let

$$
\operatorname {a c c e p t} \left(\operatorname {F o r k} ^ {\mathcal {C}}\right) := \Pr \left[ \alpha \neq \bot : X \leftarrow \mathrm {I G}, \alpha \leftarrow \operatorname {F o r k} ^ {\mathcal {C}} (X) \right].
$$

Then,  $\operatorname{accept}(\operatorname{Fork}^{\mathcal{C}})$  is bounded by

$$
\operatorname {a c c e p t} \left(\operatorname {F o r k} ^ {\mathcal {C}}\right) \geq \operatorname {a c c e p t} (\mathcal {C}) \cdot \left(\frac {\operatorname {a c c e p t} (\mathcal {C})}{r} - \frac {1}{| H |}\right).
$$

A modified forking lemma. We will employ a slight modification of the forking experiment, and give a corollary on how it impacts the accepting probability of its output.

The modification to the forking experiment is natural; intuitively, we add an additional abort condition if any of the values  $h_1, \ldots, h_r, h_j', \ldots, h_r'$  collide. Because there are at most  $2r$  values, and they are all sampled uniformly at random from the set  $H$ , the probability that any of them collide is at most  $2r^2 / |H|$ . By considering this case here, we can be sure that such collisions are considered in our proof of security for Arctic.

###### Corollary 1.

Let $\mathsf{Fork}^{\mathcal{C}}_{m}$ be the forking experiment in Figure 1. Then using the notation of Lemma 1 we have

$\mathsf{accept}(\mathsf{Fork}^{\mathcal{C}}_{m})$ $\geq\mathsf{accept}(\mathsf{Fork}^{\mathcal{C}})-\Pr[\mathsf{BadHashEvent}]$ (1)
$\geq\mathsf{accept}(\mathsf{Fork}^{\mathcal{C}})-2r^{2}/|H|$

Where $\mathsf{BadHashEvent}$ denotes the event that $\mathsf{Fork}^{\mathcal{C}}_{m}$ returns $\bot$ due to the boxed lines in Figure 1.

### 3.4 Deterministic Threshold Signature Schemes

We begin with the definition of a deterministic threshold signature scheme, and then define the notion of unforgeability. We build upon standard definitions and notions of unforgeability for threshold signatures in the literature *[16, 28, 30, 26]*.

Intuitively, our definition for deterministic threshold signature schemes is identical to that of randomized threshold signature schemes, with the exception that in the deterministic setting, the signing algorithms are deterministic and the signer does not maintain state between rounds. Furthermore, each signing round in the deterministic setting is given as input the message $m$, coalition of signers $C$, and secret signing key share $\mathsf{sk}_{i}$.

###### Definition 3 (Deterministic Threshold Signatures).

A deterministic threshold signature scheme $\mathsf{DT}$ with an interactive signing protocol consisting of $r$ rounds is a tuple of PPT algorithms $\mathsf{TS}=(\mathsf{Setup},\mathsf{KeyGen},(\mathsf{Sign}_{1},\ldots,\mathsf{Sign}_{r}),\mathsf{Combine},\mathsf{Verify})$, defined as follows:

- $\mathsf{Setup}(1^{\lambda})\to\mathsf{par}$: Accepts as input a security parameter $\lambda$ and outputs public parameters $\mathsf{par}$, which are then implicitly given as input to all other algorithms.
- $\mathsf{KeyGen}(n,t,\mu)\to(\mathsf{pk},\{\mathsf{pk}_{i},\mathsf{sk}_{i}\}_{i\in[n]})$: A probabilistic algorithm that takes as input the total number of possible signers $n$, the corruption threshold $t$, and the minimum number of participating signers $\mu$. Outputs the public key $\mathsf{pk}$ representing the set of $n$ signers, the set $\{\mathsf{pk}_{i},\mathsf{sk}_{i}\}_{i\in[n]}$ of public and secret key shares for each signer.
- $(\mathsf{Sign}_{1},\ldots,\mathsf{Sign}_{r})\to\{\rho_{1}^{(k)},\ldots\rho_{r}^{(k)}\}_{k\in C}$: A set of deterministic algorithms where each algorithm represents a single stage in an interactive signing protocol performed by signing party $k\in[n]$ in a signing set $C\subseteq[n],|C|\geq\mu$ with respect to a message $\mathsf{m}$, defined as follows:
$\rho_{1}^{(k)}\leftarrow\mathsf{Sign}_{1}(k,\mathsf{sk}_{k},\mathsf{m},C),\quad\ldots,\quad\rho_{r}^{(k)}\leftarrow\mathsf{Sign}_{r}(k,\mathsf{sk}_{k},\mathsf{m},C,\{\rho_{r-1}^{(i)}\}_{i\in C})$
where $\rho_{1}^{(k)},\ldots,\rho_{r}^{(k)}$ are protocol messages produced by party $k\in C$.
- $\mathsf{Combine}(\mathsf{pk},\mathsf{m},C,\{(\rho_{1}^{(i)},\ldots,\rho_{r}^{(i)})\}_{i\in C})\to\sigma$: A deterministic algorithm that takes as input the public key $\mathsf{pk}$, the message $\mathsf{m}$, the set of signers $C$, and the set of protocol messages sent by each party during the $\mathsf{Sign}_{1},\ldots,\mathsf{Sign}_{r}$ signing stages, and outputs a joint signature $\sigma$.
- $\mathsf{Verify}(\mathsf{pk},\mathsf{m},\sigma)\to 0/1$: A deterministic algorithm that takes as input the public key $\mathsf{pk}$, a message $\mathsf{m}$, and signature $\sigma$ and outputs $1$ to indicate accept if the signature verifies; otherwise, it outputs $0$ to indicate reject.

###### Remark 1 (Distributed key generation).

Our definition assumes a centralized key generation algorithm KeyGen to generate the public key pk and public key shares $\{\mathsf{pk}_{i},\mathsf{sk}_{i}\}_{i\in[n]}$. However, our scheme and proofs can be adapted to use a fully decentralized distributed key generation protocol (DKG).

###### Remark 2 (Deferring the Choice of Coalition to Later Rounds).

Our definition assumes that the coalition of signers $C$ is provided in the first round of signing $\mathsf{Sign}_{1}$. However, some constructions (as is the case with Arctic) may defer the choice of $C$ to later rounds.

##### Correctness.

A deterministic threshold signature scheme DT is *correct* if for all security parameters $\lambda$, all $1\leq t\leq\mu\leq n$, all $C\subseteq[n]$ such that $\mu\leq|C|\leq n$, and for all messages $m\in\{0,1\}^{*}$, the following relation holds:

$\mathsf{DT.Verify}(\mathsf{pk},\mathsf{m},\sigma)=1,\text{ for }$
$(\mathsf{pk},\{\mathsf{pk}_{i},\mathsf{sk}_{i}\}_{i\in[n]})\stackrel{{\scriptstyle\iota}}{{{\leftarrow}}}\mathsf{DT.KeyGen}(n,t,\mu),\text{ where }$
$\rho_{1}^{(k)}\leftarrow\mathsf{DT.Sign}_{1}(k,\mathsf{sk}_{k},\mathsf{m},C),\ldots,\rho_{r}^{(k)}\leftarrow\mathsf{DT.Sign}_{r}(k,\mathsf{sk}_{k},\mathsf{m},C,\{\rho_{r-1}^{(i)}\}_{i\in C}),\text{ and }$
$\sigma\leftarrow\mathsf{DT.Combine}(\mathsf{pk},\mathsf{m},C,\{\rho_{1}^{(i)},\ldots,\rho_{r}^{(i)}\}_{i\in C})$

#### Unforgeability

We present a game-based definition of unforgeability for a deterministic threshold signature scheme in Figure 2. This notion of unforgeability is analogous to the standard notion of chosen message attack (EUF-CMA) for standard signature schemes *[31]*. We present the adversary as a static adversary, which is allowed to corrupt at most $t-1$ signers.

In the static unforgeability game for a deterministic threshold signature scheme, the environment allows the adversary to sample the corrupt parties $\mathsf{corrupt}\subset[n]$. If the set of corrupt parties is smaller than the corruption threshold $t$, it derives the set of honest parties $\mathsf{honest}\subseteq[n]$ as the remaining parties in $[n]$. The environment then runs KeyGen to derive the joint public key pk representing the set of $n$ signers, the set of public key shares $\{\mathsf{pk}_{i}\}_{i\in[n]}$, and the secret key shares $\{\mathsf{sk}_{i}\}_{i\in[n]}$. The adversary is then run on input $n,t,\mu,\mathsf{par},\mathsf{pk}$, $\{\mathsf{pk}_{i}\}_{i\in[n]}$, and the corrupt signing key shares $\{\mathsf{sk}_{j}\}_{j\in\mathsf{corrupt}}$. The adversary can then query any honest signers $k\in\mathsf{honest}$ of its choosing at each step in the signing protocol, and has full power over choosing the set of signers $C$ and the message $\mathsf{m}$. Additionally, the adversary may query the signing round oracles in arbitrary order. However, unlike in the randomized setting, the environment for a deterministic threshold signature scheme does not maintain any session identifiers, or state about ongoing signing sessions. The adversary wins if it can produce a valid forgery $\sigma^{*}$ with respect to the joint public key $\mathsf{pk}$ on a message $\mathsf{m}^{*}$ that has not been queried to $\mathcal{O}^{\mathsf{Sign}_{r}}$ (i.e., in the final round of signing). Importantly, this definition allows the adversary to be *rushing*, meaning it can wait to produce its outputs after having seen the honest outputs first. The adversary may also be *concurrent*, meaning it can open simultaneous signing sessions at once, or choose not to complete a signing session.

|  ExpntDT_A(λ,n,t,μ) | OSign1(k,m,C)  |
| --- | --- |
|  par←DT.Setup(1λ) | // k denotes the participant identifier  |
|  Q←∅ // set of queried messages | ρ1(k)←DT.Sign1(k,skk,m,C)  |
|  (corrupt,stA)←A(par,n,t,μ) | return ρ1(k)  |
|  if |corrupt|≥t | :  |
|  return ⊥ | OSignj(k,m,C,{ρj(i)i∈C})  |
|  honest←[n] \ corrupt | // for j∈{2,...,r-1}  |
|  (pk,{pk_i,sk_i}n=1)←DT.KeyGen(n,t,μ) | ρj(k)←DT.Signi(k,skk,m,C,{ρj(i)i∈C})  |
|  in←(pk,{pk_i}n=1,{sk_j}j∈corrupt,stA) | return ρj(k)  |
|  (m*,σ*)←AOSigni,i∈[r] (in) | :  |
|  if m*∉Q ∧ DT.Verify(pk,m*,σ*) = 1 | :  |
|  return 1 | OSignr(k,m,C,{ρr(i)i∈C})  |
|  return 0 | Q←Q∪{m}  |
|   | ρr(k)←DT.Signr(k,skk,m,C,{ρr(i)i∈C})  |
|   | return ρr(k)  |

Fig. 2: Unforgeability game for a deterministic threshold signature scheme DT with a  $r$ -round signing protocol. The game assumes a static adversary that picks the players it corrupts at the beginning of the game. The public parameters par are implicitly given as input to all algorithms, and  $\rho_1^{(k)}, \ldots, \rho_r^{(k)}$  represent protocol messages sent by participant  $k$  throughout the interactive signing protocol.

Definition 4 (Unforgeability). Let the advantage of a static adversary  $\mathcal{A}$  playing the unforgeability game against a deterministic threshold signature scheme DT as defined in Figure 2 be as follows:

$$
\operatorname {A d v} _ {\mathrm {D T}, \mathcal {A}} ^ {\mathrm {u f}} (\lambda , n, t, \mu) = \left[ \Pr \left[ \operatorname {E x p} _ {\mathrm {D T}, \mathcal {A}} ^ {\mathrm {u f}} (\lambda , n, t, \mu) = 1 \right] \right]
$$

A deterministic threshold signature scheme DT is unforgeable if for all PPT adversaries  $\mathcal{A}$ ,  $\mathrm{Adv}_{\mathrm{DT},\mathcal{A}}^{\mathrm{uf}}(\lambda, n, t, \mu)$  is negligible in  $\lambda$ , for all  $n, t, \mu \in \mathbb{N}$ , such that  $t \leq n$ .

# 4 Verifiable Pseudorandom Secret Sharing

We now introduce an extension to pseudorandom secret sharing that we call a Verifiable Pseudorandom Secret Sharing (VPSS) scheme. We begin by motivating the need for a VPSS, and then formally define VPSS and its security properties. Finally, in Section 4.3, we give a concrete VPSS construction,  $\mathsf{VPSS}_1$ , that we later use as a building block for Arctic.

4.1 Motivation

A verifiable random function (VRF) *[43]* is a keyed PRF, such that the PRF can be evaluated using only knowledge of a secret key, but is publicly verifiable given a public key. In particular, in addition to outputting the evaluation of the VRF, it also outputs a zero-knowledge proof that the VRF was evaluated correctly. A *distributed* VRF *[19, 24, 44]* allows the evaluation algorithm to be partitioned among a set of participants, all of whom are equally trusted.

However, the use case of generating deterministic nonces for threshold Schnorr signatures presents a slightly different challenge. Instead of directly verifying that the nonce $r_{i}$ was correctly generated, we instead need to verify correctness in zero-knowledge, with respect to a commitment $R_{i}=g^{r_{i}}$. One approach in the literature is to employ non-algebraic pseudorandom functions to generate the nonce, and then prove in zero-knowledge the correctness of the corresponding commitment *[25, 46]*. Unfortunately, generating such zero-knowledge proofs requires higher computational and complexity overhead.

We instead take a different approach towards verifying that a distributed pseudorandom function was correctly performed by a set of parties. Instead of each participant outputting a zero-knowledge proof that they individually performed the action correctly, we observe that the correctness of the evaluation can instead be *collectively* publicly verified, assuming some threshold of honest participants. Such an observation leads naturally to employing a pseudorandom secret sharing scheme *[15]*, which is categorized by all parties holding a set of secret key shares, and individually and non-interactively being able to generate secret shares of additional pseudorandom values. We show that pseudorandom secret sharing schemes have a natural extension to the publicly verifiability setting. Finally, we give a concrete and efficient VPSS scheme, where all participants can publish commitments to their generated shares, and perform polynomial interpolation over elements in the public domain to ensure correctness.

We next build upon these observations by formalizing the notion of a verifiable pseudorandom secret sharing scheme.

### 4.2 VPSS Definition and Notions of Security

We now present an extension on pseudorandom secret sharing that we call a *Verifiable Pseudorandom Secret Sharing* (VPSS) scheme. In particular, a VPSS defines an additional verify algorithm to ensure that the pseudorandom function was performed correctly by collectively verifying outputs by all participants.

###### Definition 5.

A *Verifiable Pseudorandom Secret Sharing* scheme is the tuple of algorithms $\textsf{VPSS}=(\textsf{Setup},\textsf{KeyGen},\textsf{Gen},\textsf{Verify},\textsf{Agg},\textsf{Recover})$, such that:

- $\textsf{Setup}(1^{\lambda})$: Accepts as input the security parameter $\lambda$, and outputs public parameters $\textsf{par}$, where $\textsf{par}$ is given as implicit input to all other algorithms.
- $\textsf{KeyGen}(n,t,\mu)\rightarrow\bot/(\textsf{sk}_{1},\ldots,\textsf{sk}_{n})$: A probabilistic algorithm that accepts as input the total number of participants $n$, a corruption threshold $t$, and

the minimum number of parties $\mu$ required to participate in $\mathsf{Gen}$. On failure, output $\bot$, otherwise, output $n$ secret keys, one for each of the $n$ participants.
- $\mathsf{Gen}(k,\mathsf{sk}_{k},w)\to(d_{k},D_{k})$: A deterministic algorithm that accepts as input a participant identifier $k$, the secret key for that participant $\mathsf{sk}_{k}$, and some input $w$. Generates the pseudorandom secret share $d_{k}\in\mathcal{P}$ from some domain $\mathcal{P}$, using $\mathsf{sk}_{k}$ as the random seed and $w$ as the corresponding input. Then, generates its public commitment $D_{k}\in\mathcal{O}$ to $d_{k}$ from some domain $\mathcal{O}$. Outputs $(d_{k},D_{k})$.
- $\mathsf{Verify}(t,\mu,C,\{D_{j}\}_{j\in C})\to\{0,1\}$: A deterministic algorithm that accepts as input the corruption threshold $t$, the minimum number of participants $\mu$, a coalition of participants $C$ such that $|C|\geq\mu$, and a set of commitments to pseudorandom secret shares $(D_{j})_{j\in C}$ for that coalition. Outputs $1$ to indicate that the secret sharing was correctly performed, otherwise, output $0$.
- $\mathsf{Agg}(t,\mu,C,\{D_{j}\}_{j\in C})\to D$: A deterministic algorithm that accepts as input the corruption threshold $t$, the minimum number of participants $\mu$, a coalition of participants $C$ such that $|C|\geq\mu$, and the set of commitments to pseudorandom secret shares. Outputs the commitment to the aggregated pseudorandom secret $D$.
- $\mathsf{Recover}(t,\mu,C,\{d_{j}\}_{j\in C})\to\bot/(d,D)$: A deterministic algorithm that accepts a corruption threshold $t$, the minimum number of participants $\mu$, a coalition $C\subseteq[n],|C|\geq\mu$, and a set of shares $\{d_{j}\}_{j\in C}$. It outputs $\bot$ if $C\nsubseteq[n]$, $|C|<\mu$, or if the shares are inconsistent. Otherwise, it recovers $d$ using the set of shares, derives the corresponding commitment $D$, and outputs $(d,D)$.

##### Correctness

A VPSS is correct if for all $\lambda$, possible inputs $w$ and choices of $t,\mu,n\in\mathbb{N}$ where $t\leq\mu\leq n$, when $\mathsf{par}\leftarrow\mathsf{VPSS}.{\mathsf{Setup}}(1^{\lambda})$ and $(\mathsf{sk}_{1},\ldots,\mathsf{sk}_{n})\xleftarrow{1}{\mathsf{VPSS}.{\mathsf{KeyGen}}(n,t,\mu)}$, there exists $d\in\mathcal{P},D\in\mathcal{O}$ such that Equation 2 holds:

$\text{for all }i\in[n]\text{ and for all }C_{\ell}\subseteq[n],|C_{\ell}|\geq\mu,\text{ when}$ (2)
$\mathsf{VPSS}.{\mathsf{Gen}}(i,\mathsf{sk}_{i},w)\to(d_{i},D_{i}),\text{ then}$
$\mathsf{VPSS}.{\mathsf{Verify}}(t,\mu,C_{\ell},\{D_{j}\}_{j\in C})=1,\text{ and}$
$\mathsf{VPSS}.{\mathsf{Recover}}(t,\mu,C_{\ell},(d_{j})_{j\in C})\to(d,D),\text{ and}$
$\mathsf{VPSS}.{\mathsf{Agg}}(t,\mu,C_{\ell},(D_{j})_{j\in C})\to D$

Security. Similarly to verifiable random functions *[19, 24, 43, 44]*, we say that a VPSS is *secure* if it is unique, verifiable, and pseudorandom.

##### Uniqueness

Intuitively, a VPSS is unique if it produces exactly one commitment to a (pseudorandom) value for each input $w$, regardless of the choice of coalition. More specifically, picking different coalitions should always result in the same commitment $D$, when the input $w$ remains the same. We show the VPSS uniqueness experiment in Figure 3.

![img-1.jpeg](img-1.jpeg)
Fig. 3: Uniqueness game for a VPSS.

In the uniqueness experiment, the adversary is allowed to query honest participants for shares and commitments on inputs of its choosing. The adversary then outputs the evaluation input  $w$ , two coalitions  $C, C'$ , and two sets of commitments  $(D_i)_{i \in C \cap \text{corrupt}}$ ,  $(D_i')_{i \in C' \cap \text{corrupt}}$ .

The environment then derives the honest players' shares and commitments on  $w$ , producing  $(d_j, D_j)_{j \in \text{honest}}$ . After deriving the sets  $S_1 \gets (D_i)_{i \in C \cap \text{corrupt}} \cup (D_i)_{i \in C \cap \text{honest}}$  and  $S_2 \gets (D_i')_{i \in C' \cap \text{corrupt}} \cup (D_i)_{i \in C' \cap \text{honest}}$ , the adversary loses if the verification algorithm outputs 0 on either set. If both sets verify, the adversary wins if the corresponding commitments for each set are not equal.

We define uniqueness for a VPSS more formally in Definition 6.

Definition 6. Let the advantage of an adversary  $\mathcal{A}$  playing the uniqueness game as defined in Figure 3 be as follows:

$$
\operatorname {A d v} _ {\mathrm {V P S S}, \mathcal {A}} ^ {\mathrm {u q}} (\lambda , n, t, \mu) = \left| \Pr \left[ \operatorname {E x p} _ {\mathrm {V P S S}, \mathcal {A}} ^ {\mathrm {u q}} (\lambda , n, t, \mu) = 1 \right] \right|
$$

A VPSS VPSS is unique if for all PPT adversaries  $\mathcal{A}$ ,  $\mathrm{Adv}_{\mathrm{VPSS},\mathcal{A}}^{\mathrm{unq}}$  is a negligible function of  $\lambda$ , for  $n,t,\mu \in \mathbb{N}$ ,  $t \leq \mu \leq n$ .

![img-2.jpeg](img-2.jpeg)
Fig. 4: Verifiability game for a VPSS.

Verifiability. Intuitively, a VPSS is verifiable if given a set of commitments to shares from a coalition of participants, the verify algorithm will detect if some subset of players deviated from the protocol. We show the VPSS verifiability experiment in Figure 4.

In the experiment, the adversary is allowed to query honest participants for shares and commitments on inputs of its choosing. The adversary then outputs a coalition  $C$ , a VPSS input  $w$ , and a set of commitments  $(D_{i})_{i\in C\cap \text{corrupt}}$ . The environment then follows the protocol, deriving both the corrupt and honest players' commitments on  $w$ , and producing  $(D_j')_{j\in C}$ . The adversary loses if its output is identical to the honestly derived commitments for the corrupted players. Then, the environment checks if the set of commitments  $(D_{i})_{i\in C\cap \text{corrupt}} \cup (D_j')_{j\in C\cap \text{honest}}$  are valid with respect to  $C$  and  $w$ . If so, the adversary wins, otherwise, it loses.

We define verifiability for a VPSS more formally in Definition 7.

Definition 7. Let the advantage of an adversary  $\mathcal{A}$  playing the verifiability game as defined in Figure 4 be as follows:

$\mathsf{Adv}_{\mathsf{VPSS},\mathcal{A}}^{\mathsf{verf}}(\lambda ,n,t,\mu) = \left|\Pr \left[\mathsf{Exp}_{\mathsf{VPSS},\mathcal{A}}^{\mathsf{verf}}(\lambda ,n,t,\mu) = 1\right]\right|$

A VPSS VPSS is verifiable if for all PPT adversaries  $\mathcal{A}$ ,  $\mathsf{Adv}_{\mathsf{VPSS},\mathcal{A}}^{\mathsf{verf}}$  is a negligible function of  $\lambda$ , for  $n, t, \mu \in \mathbb{N}$ ,  $t \leq \mu \leq n$ .

![img-3.jpeg](img-3.jpeg)
Fig. 5: Pseudorandomness game for a VPSS.

Pseudorandom. Intuitively, a VPSS is pseudorandom if the adversary has negligible advantage distinguishing between a real VPSS output from one that is randomly sampled. We show the VPSS pseudorandomness experiment in Figure 5.

In the pseudorandomness experiment, the environment begins by picking a random bit  $b \gets \{0,1\}$ . It then performs key generation, and initializes the adversary with the public parameters and the corrupted parties' secret key shares.

The adversary is allowed to query  $\mathcal{O}^{\mathrm{Gen}}$  on honest participants for shares and commitments on inputs of the adversary's choosing. If  $b = 0$ , the environment responds by following the VPSS protocol. If  $b = 1$ , the environment first checks to see if it has responded to the query before, replying with the response if so. Otherwise, it uses a simulating algorithm SimGen to simulate generating honest players' shares and commitments. The details of SimGen depend on the specifics of the VPSS construction.

The adversary outputs  $(b', w^*, C^*, \{D_j\}_{j \in \text{corrupt}})$ , where  $b'$  is the adversary's guess for the value of  $b$ , and  $(w^*, C^*, \{D_j\}_{j \in \text{corrupt}})$  corresponds to one of the sessions the adversary initiated with  $\mathcal{O}^{\text{Gen}}$ .

$b=1$, the environment checks if $(w^{*},C^{*},\{D_{j}\}_{j\in\textsc{corrupt}})$ corresponds to an existing session and is consistent with the honest parties’ commitments, outputting a random coin if it does not. Otherwise, the environment next checks that the aggregated commitment $\mathsf{VPSS.Agg}(t,\mu,C^{*},\{D_{k}\}_{k\in C^{*}})$ is equal to the simulated commitment $D$ for that session, if the check does not hold, the environment outputs $1$.

Otherwise, the environment checks if the adversary’s guess $b^{\prime}$ is equal to $b$; outputting $1$ if the check holds, otherwise, outputting $0$.

We define pseudorandomness for a VPSS more formally in Definition 8.

###### Definition 8.

Let the advantage of an adversary $\mathcal{A}$ playing the pseudorandomness game as defined in Figure 5 be as follows:

$\mathsf{Adv}^{\textup{psdr}}_{\mathsf{VPSS},\mathcal{A}}(\lambda,n,t,\mu)=\big{|}\Pr[\mathsf{Exp}^{\textup{psdr}}_{\mathsf{VPSS},\mathcal{A}}(\lambda,n,t,\mu)=1]-1/2\big{|}$

A VPSS $\mathsf{VPSS}$ is pseudorandom if for all PPT adversaries $\mathcal{A}$, $\mathsf{Adv}^{\textup{psdr}}_{\mathsf{VPSS},\mathcal{A}}$ is a negligible function of $\lambda$, for $n,t,\mu\in\mathbb{N}$, $t\leq\mu\leq n$.

### 4.3 $\mathsf{VPSS}_{1}$, A Concrete Verifiable Pseudorandom Secret Sharing Scheme

We now define a concrete VPSS that we call $\mathsf{VPSS}_{1}$, that builds upon the pseudorandom secret sharing scheme as defined by Cramer, Damgård, and Ishai *[15]*. However, $\mathsf{VPSS}_{1}$ additionally defines a verify algorithm that is publicly verifiable (assuming the authenticity of the inputs), as well as an algorithm to combine participant commitments.

As a reminder, the pseudorandom secret sharing scheme by Cramer, Damgård, and Ishai *[15]* itself builds on replicated secret sharing *[32]*. However, Cramer et al. define a mechanism to non-interactively convert shares of a replicated secret sharing scheme to shares of a Shamir-secret-shared value. They then show how this mechanism can be used as a distributed pseudorandom function; i.e., given a replicated secret sharing of a random value, participants can generate Shamir secret shares of an unbounded number of pseudorandom values, without interaction.

We extend this pseudorandom secret sharing scheme by Cramer et al. and additionally define a public verifiability mechanism. More specifically, we define a Verify function that uses the set of participants’ commitments to ensure all participants performed the evaluation step in a consistent manner. Such consistency checks have been used in prior literature *[4]*, but our check is performed over commitments to secret shares, as opposed to verifying the shares directly. We prove that $\mathsf{VPSS}_{1}$ is secure assuming a cryptographically secure hash function, when at minimum $2t-1$ parties participate in evaluation and up to $t-1$ participants are corrupted (honest majority).

We now define $\mathsf{VPSS}_{1}$ in more detail. We show KeyGen for $\mathsf{VPSS}_{1}$ as a centralized operation, but this operation can easily be decentralized. $\mathsf{VPSS}_{1}$ is additionally defined with respect to a hash function $\mathsf{H}$, where $\mathsf{H}:\mathbb{Z}_{q}\times\{0,1\}^{*}\to\mathbb{Z}_{q}$ is a cryptographically secure hash function.

- $\mathsf{VPSS}_1.\mathsf{Setup}(1^\lambda)$: Accepts as input the security parameter $\lambda$, outputs public parameters $\mathsf{par} = (\mathbb{G}, q, g)$, where $(\mathbb{G}, q, g)$ is generated by $\mathsf{GroupGen}(1^\lambda)$. $\mathsf{par}$ is given as an implicit input to all other algorithms.

- $\mathsf{VPSS}_1[\mathsf{H}].\mathsf{KeyGen}(n, t, \mu) \to \bot / (\mathsf{sk}_1, \ldots, \mathsf{sk}_n)$: On input the total number of participants $n$, the corruption threshold $t$, and minimum number of participants $\mu$, the dealer performs replicated secret sharing of a random secret $\mathsf{sk} \in \mathbb{Z}_q$ [15], following the following steps:

1. First, the dealer checks if $\mu \geq 2t - 1$; if the check fails, output $\bot$.

2. Let $A$ be the set such that $A = \binom{[n]}{t-1}$, and let $\gamma$ be the size of $A$; i.e., $\gamma = |A| = \binom{n}{t-1}$.

3. Generate $\gamma$ secret shares $\{\phi_i\}_{i\in [\gamma]}$, by sampling $\phi_i \stackrel{\mathrm{s}}{\leftarrow} \mathbb{Z}_q, i \in [\gamma]$. Implicitly, the secret $\mathsf{sk}$ is such that $\mathsf{sk} = \sum_{i\in \gamma} \phi_i$.

4. Initialize empty sets $\mathsf{sk}_1 = \emptyset, \ldots, \mathsf{sk}_n = \emptyset$.

5. For each set $a_i \in A, i \in [\gamma]$ and each participant identifier $j \in [n] \setminus a_i$, append $\mathsf{sk}_j \gets \mathsf{sk}_j \cup \{(a_i, \phi_i)\}$.

6. Output $(\mathsf{sk}_1, \ldots, \mathsf{sk}_n)$. Each $\mathsf{sk}_j$ is a set of size $\delta = \binom{n-1}{t-1}$, consisting of the tuples $(a_i, \phi_i)$. Each $a_i$ is itself a set, such that $a_i \subset [n]$ and $|a_i| = t - 1$, where if $a_i \in \mathsf{sk}_j$, then $j \notin a_i$.

- $\mathsf{VPSS}_1[\mathsf{H}].\mathsf{Gen}(k,\mathsf{sk}_k,w)\to (d_k,D_k)$: On input participant identifier $k\in [n]$, secret key share $\mathsf{sk}_k$, and input $w\in \{0,1\}^*$, perform the following steps:

1. Parse $\{(a_i,\phi_i)\}_{i\in [\delta ]}\gets \mathsf{sk}_k$.

2. For each set $a_i$, let $L_{a_i}'(x)$ be the degree $t - 1$ polynomial defined by the set $a_i$, as given in Equation 3:

$$
L _ {a _ {i}} ^ {\prime} (x) = \prod_ {j \in a _ {i}} \frac {j - x}{j} \tag {3}
$$

Note that $L_{a_i}'(j) = 0$ for all $j \in a_i$ and $L_{a_i}'(0) = 1$.

3. Obtain the share via Equation 4:

$$
d _ {k} \leftarrow \sum_ {i \in [ \delta ]} \mathrm {H} \left(\phi_ {i}, w\right) \cdot L _ {a _ {i}} ^ {\prime} (k) \tag {4}
$$

Note that the $L_{a_i}'(k)$ values can be precomputed by participant $k$, as they are independent of $w$, so this step requires $\delta$ evaluations of $\mathsf{H}$ and $\delta$ multiplications and additions in $\mathbb{Z}_q$.

4. Derive the commitment to $d_{k}$ as $D_{k} \gets g^{d_{k}}$.

5. Output $(d_k, D_k)$.

- $\mathsf{VPSS}_1[\mathsf{H}].\mathsf{Verify}(t,\mu ,C,\{D_j\}_{j\in C})\to \{0,1\}$: Perform the following steps:

1. If $|C| \neq \mu$, return 0.

22

2. Otherwise, define $B = (B_0, B_1, \ldots, B_{|C|-1})$ to be the tuple of $|C|$ commitments to coefficients of a polynomial $f$ defined "in the exponent," where each $D_i, i \in C$ is a commitment to a point on the same polynomial $f$. Each $B_i$ can be derived via Equation 5:

$$
B_i = \prod_{j \in C} D_j^{L_{j,i}} \tag{5}
$$

where $L_{j,i}$ is the coefficient of the $i^{\text{th}}$ term $x^i$ of the $j^{\text{th}}$ Lagrange polynomial $L_j(x)$ for the coalition $C$, as described in Section 3.1.

3. Let $\ell = |C| - t$. The verifier now checks that all participants followed the protocol honestly, by checking in the exponent that their shares lie on a polynomial of degree $t - 1$. In particular, the verifier ensures that $B$ is a commitment to a polynomial of degree at most $t - 1$, by checking the last $\ell$ entries in $B$ are equal to the identity of $\mathbb{G}$:

$$
B_{t-1+i} = I_{\mathbb{G}}, \forall i \in [\ell] \tag{6}
$$

4. If the check in Equation 6 holds, output 1, otherwise, output 0.

- $\mathsf{VPSS}_1[\mathsf{H}].\mathsf{Agg}(t,\mu,C,\{D_j\}_{j\in C}) \to D$: Accepts as input the corruption threshold $t$, minimum participants $\mu$, the coalition $C$, and set of (verified) commitments $\{D_j\}_{j\in C}$. The commitments can be combined as in Equation 7, where the coefficients $\lambda_j$ are determined by the coalition $C$.

$$
D \leftarrow \prod_{j \in C} D_j^{\lambda_j} = B_0 \tag{7}
$$

Output $D$

- $\mathsf{VPSS}_1[\mathsf{H}].\mathsf{Recover}(t,\mu,C,\{d_j\}_{j\in C}) \to \bot/(d,D)$: Receives as input the corruption threshold $t$, the minimum number of participants $\mu$, the coalition $C$, and the set of pseudorandom secret shares $\{d_j\}_{j\in C}$. Performs the following steps:

1. If $|C| &lt; \mu$, or $C \not\subset [n]$, output $\bot$.

2. For each $d_j, j \in C$, derive $D_j \gets g^{D_j}$.

3. If $\mathsf{VPSS}_1.\mathsf{Verify}(t,\mu,C,\{D_j\}_{j\in C}) \neq 1$, then output $\bot$.

4. Otherwise, the shares can be combined as in Equation 8, where the $\lambda_j$ are determined by the coalition $C$.

$$
d \leftarrow \sum_{j \in C} d_j \cdot \lambda_j \tag{8}
$$

5. Derive the commitment to $d$ as $D \gets g^d$.

6. Output $(d,D)$.

3 When at least $t$ parties follow the protocol honestly, their shares will completely define this polynomial.

4 As an optimization, note that $B_1, \ldots, B_{t-1}$ need not be computed.

23

##### Correctness.

By correctness of pseudorandom secret sharing as defined by *Cramer et al. [15]*, when key generation is honestly performed, the Gen algorithm produces secret shares $d_{i}=f(i)$ that are points on a degree $t-1$ Shamir secret sharing polynomial $f(x)=\sum_{i\in[\gamma]}\mathsf{H}(\phi_{i},w)L^{\prime}_{a_{i}}(x)$. The combined commitment $d=\sum_{i\in C}d_{i}\lambda_{i}$ for $C\subseteq[n],|C|\geq 2t-1$ is a commitment to a pseudorandom secret value $d=f(0)=\sum_{i\in[\gamma]}\mathsf{H}(\phi_{i},w)$. Agg and Recover simply perform polynomial interpolation of these values with respect to the set of participants, and so are likewise correct.

Verify is correct because honest parties output commitments $D_{i}=g^{f(i)}$ for the degree $t-1$ polynomial $f$ defined above. Therefore, if all parties are honest, the coefficients of $x^{i}$, …, $x^{|C|-1}$ of $f(x)$ will be $0$, so the commitments $B_{t}$, …, $B_{|C|-1}$ to those coefficients will be $I_{\mathbb{G}}$ (the identity element of $\mathbb{G}$), and so Verify will output $1$.

##### Security.

$\mathsf{VPSS}_{1}$ is verifiable, unique, and pseudorandom. We give the corresponding proofs in the full version of this work *[37, App.B]*.

## 5 Arctic, A Deterministic and Stateless Two-Round Threshold Schnorr Signature Scheme

We now introduce Arctic, an efficient, two-round, deterministic threshold Schnorr signature scheme for moderately sized groups of participants that does not require participants to keep state between rounds of the signing protocol. As a building block, Arctic uses $\mathsf{VPSS}_{1}$ to generate nonces deterministically, and to verify that all other participants followed the protocol honestly. Arctic is secure assuming fewer than $t$ participants are corrupted, and at least $\mu$ participated in the signing protocol, where $\mu\leq n$ but $\mu\geq 2t-1$.

###### Remark 3 (Distributed Key Generation)

The Arctic construction given in Figure 6 assumes a centralized key generation procedure; however, using a distributed key generation (DKG) scheme is equally possible. We discuss one possible DKG in Section 5.2.

##### Remark 4 (Requirement of Authenticated Channels)

We require that the messages exchanged between participants in the Arctic construction shown in Figure 6 be sent over authenticated channels; i.e., messages must be authenticated and verifiable as having come from their purported senders. Otherwise, an adversary can simply pick contributions that are consistent with a single honest party’s $R_{i}$, which would result in a valid input for $\mathsf{VPSS}_{1}$.Verify. Note that we do *not* assume the authenticated channel maintains any session identifiers or state about messages; Arctic remains secure even if the adversary were to replay old authenticated messages. However, if a participant receives an unauthenticated message, we require that the participant aborts.

|  Setup(1λ) | Sign2(k,skk,m,C,{yj,Rj})j∈C)  |
| --- | --- |
|  1: (G,q,g)← GroupGen(1λ) | 1: return ⊥ if |C| < 2t-1  |
|  2: par← ((G,q,g),H1,H2,H3) | 2: (skk(1),skk(2),pk)← skk  |
|  3: return par | 3: y'← H2(pk,m)  |
|  4: // par is given implicitly | 4: for j ∈ C do  |
|  5: // to all other algorithms | 5: if yj ≠ y'  |
|  KeyGen(n,t,μ) | 6: return ⊥  |
|  1: // Performed by a trusted | 7: (rk,Rk')  |
|  2: // dealer, or DKG | 8: ← VPSS1[H1].Gen(k,skk(1),y')  |
|  3: if μ < 2t-1 or μ > n | 9: // Re-derive state from Sign1  |
|  4: return ⊥ | 10: if Rk' ≠ Rk  |
|  5: // Require ≥ 2t-1 signers | 11: return ⊥  |
|  6: sk ← Zq; pk ← gsk | 12: input ← (t,μ,C,{Rj}j∈C)  |
|  7: {(i,sk(2))}i=1← Shamir.Share(sk,n,t) | 13: if VPSS1[H1].Verify(input) ≠ 1  |
|  8: (ski(1))n=1← VPSS1[H1].KeyGen(n,t,μ) | 14: return ⊥  |
|  9: for i ∈ {1,...,n} do | 15: R ← VPSS1[H1].Agg(input)  |
|  10: pk(2) ← gsk(2) | 16: c ← H3(R,pk,m)  |
|  11: ski ← (ski(1),ski(2),pk) | 17: zk ← rk + c · skk(2)  |
|  12: pk ← (pki(2)) | 18: return zk  |
|  13: return (pk,{ (pk1,ski)}i∈[n]) | Combine(pk,m,C,{ (yj,Rj),zj}j∈C))  |
|  Sign1(k,skk,m) | 1: input ← (t,μ,C,{Rj}j∈C)  |
|  1: (skk(1),skk(2),pk) ← skk | 2: R ← VPSS1[H1].Agg(input)  |
|  2: yk ← H2(pk,m) | 3: c ← H3(R,pk,m)  |
|  3: (rk,Rk) ← VPSS1[H1].Gen(k,skk(1),yk) | 4: // λj are Lagrange  |
|  4: return (yk,Rk) | 5: // coefficients for C  |
|   | 6: z ← ∑j∈C zjλj  |
|   | 7: if gz ≠ R·pkc  |
|   | 8: return ⊥  |
|   | 9: return σ = (R,z)  |

Fig. 6: Arctic, a deterministic threshold Schnorr signature scheme. Arctic requires that the minimum number of signing parties  $\mu \leq n$  be at minimum  $\mu \geq 2t - 1$ , where  $t$  is the tolerated corruption threshold. We further require that messages exchanged between participants are sent over an authenticated channel. Arctic builds upon the verifiable pseudorandom secret sharing scheme  $\mathsf{VPSS}_1$  defined in Section 4.3, as well as Shamir secret sharing. Verification of signatures is identical to the Schnorr verification algorithm.

5.1 The Construction

We now give more detail for each stage in Arctic; see Figure 6 for a high-level overview.

#### Key Generation.

All participants with identifiers $i\in[n]$ begin by receiving a secret signing share $\mathsf{sk}_{i}^{(2)}$ and a public signing share $\mathsf{pk}_{i}^{(2)}=g^{\mathsf{sk}_{i}^{(2)}}$. In Figure 6, we show key generation as a centralized procedure, but a DKG can likewise be used. Each $\mathsf{sk}_{i}^{(2)}$ is a $t$-of-$n$ Shamir secret sharing of the group’s joint secret key $\mathsf{sk}$; participants use these keys for signing messages. Participants also receive the public signing keys $\{\mathsf{pk}_{i}^{(2)}\}_{i\in[n]}$ for all other participants.

Each participant receives a secret $\mathsf{VPSS}_{1}$ key $\mathsf{sk}_{i}^{(1)}$ generated by performing $\mathsf{VPSS}_{1}$.KeyGen. Participants use these keys to generate nonces and commitments for each signing session. Each participant’s public key share $\mathsf{pk}_{i}$ is just one public key, where $\mathsf{pk}_{i}=\mathsf{pk}_{i}^{(2)}$. Each participant’s secret key share is the tuple $\mathsf{sk}_{i}=(\mathsf{sk}_{i}^{(1)},\mathsf{sk}_{i}^{(2)},\mathsf{pk})$.

#### Coordinator Role.

Our description of Arctic assumes an external mechanism to choose the set $C\subseteq[n]$ of signers, such that $\mu\leq|C|\leq n$. The coordinator may perform denial of service attacks, but otherwise cannot impact the security of Arctic. The coordinator can be a standalone entity, or may also be a signer as well. It is straightforward, however, to define Arctic in a peer-to-peer setting.

#### Signing.

In the first round of signing, each participant $k$ receive as input a message $\mathsf{m}$. First, each party derives $y_{k}\leftarrow\mathsf{H}_{2}(\mathsf{pk},\mathsf{m})$. To generate their nonce $r_{k}$ and commitment $R_{k}$, each participant performs $(r_{k},R_{k})\xleftarrow{\text{*}}\mathsf{VPSS}_{1}.\mathsf{Gen}(k,\mathsf{sk}_{k}^{(1)},y_{k})$. Each participant then outputs $(y_{k},R_{k})$; they do not need to keep any state.

In the second round of signing, all participants again receive as input a message $\mathsf{m}$, as well as a set $C$ representing the indices of at least $\mu$ signers, where $C\subseteq[n],|C|\geq\mu$. Additionally, participants receive the list of tuples $\{(y_{j},R_{j})\}_{j\in C}$. First, each party re-derives $y^{\prime}\leftarrow\mathsf{H}_{2}(\mathsf{pk},\mathsf{m})$. Then, each party checks the consistency of all other parties’ views of $\mathsf{m}$ by checking that for each $i\in C$, $y_{i}=y^{\prime}$. Because we require that each protocol message is authenticated by its respective party, then this check guarantees that an adversarial player cannot split the view of honest players by sending different messages or choices of coalitions. If any check fails, the party aborts.

Otherwise, if all consistency checks succeed, each participant re-derives their nonce and commitment using $\mathsf{VPSS}_{1}$, again performing $(r_{k},R_{k}^{\prime})\xleftarrow{\text{*}}\mathsf{VPSS}_{1}.\mathsf{Gen}(k,\mathsf{sk}_{k}^{(1)},y^{\prime})$. Then, the participant checks that $R_{k}^{\prime}=R_{k}$; i.e., that the commitment for $k$ in the set of commitments given as input to $\mathsf{Sign}_{2}$ indeed is the correct commitment for this party. The participant aborts if the check does not hold.

Then, each participant verifies that all other participants followed the protocol to derive their commitment, by checking $\mathsf{VPSS}_{1}.\mathsf{Verify}(t,\mu,C,(R_{j})_{j\in C})$. If the check fails, they abort the protocol.

Finally, if all of the above checks pass, each participant $k\in C$ will then derive the group commitment $R\leftarrow\mathsf{VPSS}_{1}.\mathsf{Agg}(t,\mu,C,\{R_{j}\}_{j\in C})$, and the challenge $c\leftarrow\mathsf{H}_{3}(R,\mathsf{pk},\mathsf{m})$. Finally, each participant derives their signature share $z_{k}\leftarrow r_{k}+c\cdot\mathsf{sk}_{k}^{(2)}$. Each participant outputs $z_{k}$ as its output for $\mathsf{Sign}_{2}$.

#### Combination and Verification.

To perform the Combine algorithm, the group commitment $R$ is first derived using values output by participants from $\mathsf{Sign}_{1}$. Then, the response $z$ is derived by finding $z\leftarrow\sum_{j\in C}z_{j}\lambda_{j}$, where the $\lambda_{j}$ are the Lagrange coefficients for the set $C$. The output from Combine is the Schnorr signature $\sigma=(R,z)$, which can be verified using the single-party Schnorr verification algorithm given in Definition 1.

### 5.2 Possible Extensions

#### Robustness.

Arctic as currently defined is not robust; if any party submits invalid commitments, then the output from $\mathsf{VPSS}_{1}.\mathsf{Agg}$ cannot be used. However, it is possible to extend $\mathsf{VPSS}_{1}$ to be robust, therefore also ensuring that Arctic can likewise be extended. To do so in a secure manner, the robust extension would require additional players, along with a protocol to come to consensus about which players misbehaved.

In particular, by requiring that the minimum number of participants $\mu$ be of size at least $\mu\geq 3t-2$, $\mathsf{VPSS}_{1}$ and Arctic can be securely used in a robust manner. The requirement that $\mu\geq 3t-2$ is referred to as the honest supermajority setting. We give further details on how robustness can be achieved in the full version of this work *[37, App.C]*. In this setting, $\mathsf{VPSS}_{1}.\mathsf{Verify}$ can both detect any inconsistencies as well as identify the misbehaving players. This property could likewise allow for extending Arctic to support robustness, under the same assumption that $\mu\geq 3t-2$.

#### Distributed Key Generation.

To define the signing keys $(\mathsf{pk}^{(2)},(\mathsf{sk}^{(2)}_{i})_{i=1}^{n})$, any DKG that Shamir secret shares a discrete logarithm secret key could be employed, such as the three-round DKG by Gennaro et al. *[29]*.

As one possibility to perform $\mathsf{VPSS}_{1}[\mathsf{H}_{1}].\mathsf{KeyGen}$ in a distributed manner, each subset of $n-t+1$ parties could assign one representative to generate a secret PRF key uniformly at random for that subset. The representative would then share this PRF key with all parties in their subset. To ensure security against fully malicious adversaries, parties within each subset could perform a subsequent broadcast round to compare their received inputs, to ensure consistency.

#### Completing $\mathsf{Sign}_{2}$ with $t$ Parties.

While $\mathsf{Sign}_{1}$ must be performed by $\mu\geq 2t-1$ number of signers, $\mathsf{Sign}_{2}$ requires only $t$ signers in total to perform verification of nonces and generating signature shares. Furthermore, $\mathsf{Sign}_{2}$ can be performed by $any$ $t$ number of signers, not necessarily those which participated in $\mathsf{Sign}_{1}$

5.3 Security

Correctness. In the first round of signing, participants will output nonces and commitments $(r_{i},R_{i})\leftarrow\mathsf{VPSS}_{1}.\mathsf{Gen}(i,\mathsf{sk}^{(1)}_{i},y_{i})$, for $i\in C$, where $y_{i}\leftarrow\mathsf{H}_{2}(\mathsf{pk},\mathsf{m})$, and $R_{i}=g^{r_{i}}$.

In the second round of signing, each participant receives the set of tuples $\{(y_{j},R_{j})\}_{j\in C}$. After deriving $y^{\prime}\leftarrow\mathsf{H}_{2}(\mathsf{pk},\mathsf{m})$, then the check that $y_{j}=y^{\prime}$ for each $j\in C$ will succeed, when the protocol is performed honestly.

Because $\mathsf{VPSS}_{1}$ is correct, then $\mathsf{VPSS}_{1}.\mathsf{Verify}(t,\mu,C,\{R_{j}\}_{j\in C})$ will output $1$ and the same group commitment $R=\prod_{j\in C}R_{j}^{\lambda_{j}}$ will be computed regardless of the choice of $C$. Because $\mathsf{VPSS}_{1}.\mathsf{Gen}$ is deterministic, then after deriving $y\leftarrow\mathsf{H}_{2}(\mathsf{pk},\mathsf{m})$, $\mathsf{VPSS}_{1}.\mathsf{Gen}(k,\mathsf{sk}^{(1)}_{k},y)$ will output the same $(r_{k},R_{k})$ as derived in the first round of signing.

After deriving $(r_{k},R_{k})$, all signers in a coalition $C$ output valid signature shares $z_{k}$ with respect to $R$ and challenge $c=\mathsf{H}_{3}(\mathsf{pk},R,m)$, where $z_{k}=r_{k}+c\cdot\mathsf{sk}^{(2)}_{k}$. The aggregated signature is then $\sigma=(R,z)$, where $z=\sum_{j\in C}z_{j}\cdot\lambda_{j}$. Because $\mathsf{sk}=\sum_{j\in C}\mathsf{sk}^{(2)}_{j}\lambda_{j}$, $\mathsf{pk}=g^{\mathsf{sk}}=g^{\sum_{j\in C}\mathsf{sk}^{(2)}_{j}\lambda_{j}}$, and $R=\prod_{j\in C}R_{j}^{\lambda_{j}}=g^{\sum_{j\in C}r_{j}\cdot\lambda_{j}}$, we have that $g^{z}=R\cdot\mathsf{pk}^{c}$, as required.

Unforgeability. We next demonstrate the unforgeability of Arctic, via Theorem 1.

###### Theorem 1.

Arctic is unforgeable in the ROM against a PPT adversary $\mathcal{A}$ playing the static unforgeability game as shown in Figure 2 against Arctic, assuming $\mathcal{A}$ can make up to $t-1$ corruptions, the number of honest parties is at least $t$, the discrete logarithm assumption holds, participants exchange messages over an authenticated channel, $\mathsf{VPSS}_{1}$ is a secure VPSS, and where $(n,t)\in\mathbb{N}$ are such that $\binom{n-1}{t-1}$ $=poly(n)$.

Concretely, let $\mathsf{Adv}^{\mathrm{dl}}_{\mathcal{D}}(\lambda)$ be the advantage of an adversary $\mathcal{D}$ against the discrete logarithm assumption. The advantage of $\mathcal{A}$ is bounded by

$\mathsf{Adv}^{\mathrm{uf}}_{\mathsf{Arctic},\mathcal{A}}(\lambda,n,t,\mu)\leq\sqrt{q_{r}\mathsf{Adv}^{\mathrm{dl}}_{\mathcal{D}}(\lambda)+\frac{2(q_{1}+q_{3})}{q}+\frac{3q_{r}^{2}}{q}}$

where $\mu\geq 2t-1$ and $n\geq\mu$, and where $q_{r}=q_{2}+q_{3}+2q_{s}+1$, such that $q_{s}$ is the number of times $\mathcal{A}$ is allowed to query the signing oracles, $q_{1}$ is the number of times $\mathcal{A}$ is allowed to query $\mathsf{H}_{1}$, $q_{2}$ is the number of times that $\mathcal{A}$ is allowed to query $\mathsf{H}_{2}$, and $q_{3}$ is the number of times $\mathcal{A}$ is allowed to query $\mathsf{H}_{3}$.

We give the corresponding proof for Theorem 1 in the full version of this work *[37, App.D]*.

## 6 Performance Analysis of Arctic

In this section, we analyze the performance of Arctic. In terms of the number of rounds of communication, Arctic matches the state of the art, with two, and

Fig. 7: Experimental results for Arctic
![img-4.jpeg](img-4.jpeg)
(a) Single-core wall clock time for various parameter combinations for Arctic. The times shown are the sum of the computation times for  $\mathrm{Sign}_1$ ,  $\mathrm{Sign}_2$ , and Combine. The computation time for MuSig-DN is shown for comparison.

![img-5.jpeg](img-5.jpeg)
(b) Scaling experiment, showing the wall clock time for the largest configuration  $(n, |C|, t) = (25, 25, 11)$  shown in Figure 7a as we increase the number of CPU cores.

it sends significantly less bandwidth per signature, at 65 bytes per participant. Therefore, we focus on the computational complexity.

There are two sources of potentially expensive computation: the two calls to  $\mathsf{VPSS}_1$ . Gen (one in each of  $\mathsf{Sign}_1$  and  $\mathsf{Sign}_2$ ), and the call to  $\mathsf{VPSS}_1$ . Verify in  $\mathsf{Sign}_2$ . Which one dominates depends on the parameters  $n$ ,  $t$ , and the number of participants in the signing protocol. Recall that  $C$  is a set representing the identifiers of participants in a particular signing session. Although the minimum number of participants required for signing is  $|C| \geq \mu \geq 2t - 1$ , we assume  $|C| = n$  for this analysis, to give an upper performance bound. As such, performance will be even better when  $\mu \leq |C| &lt; n$ .

$\mathsf{VPSS}_1$ . Gen primarily performs  $\delta = \binom{n-1}{t-1}$  hash computations, field multiplications, and field additions, as seen in Equation 4 (recalling that the  $L_{n_i}'(k)$  values can be precomputed).  $\mathsf{VPSS}_1$ . Verify computes  $\ell = |C| - t$ ,  $|C|$ -way multiexponentiations. When  $t$  is small, we expect the  $\mathsf{VPSS}_1$ . Verify cost to dominate, and for larger  $t$ , the  $\mathsf{VPSS}_1$ . Gen cost should dominate.

To concretely evaluate the performance of Arctic, we implemented it in Rust. We ran our implementation over all allowable combinations of parameters  $t \geq 2$ ,  $2t - 1 \leq |C| \leq n \leq 25$ , using a 4-core 3.7 GHz Intel E-2374G CPU. We measured the computation time for each of  $\mathrm{Sign}_1$ ,  $\mathrm{Sign}_2$ , and Combine, averaged over 10 signatures, for each parameter combination. In Figure 7a, we show the (single-core)

total runtime of $\mathsf{Sign}_{1}$, $\mathsf{Sign}_{2}$, and $\mathsf{Combine}$ for various combinations of parameters. We concretely measure $\mathsf{VPSS}_{1}.\mathsf{Gen}$ to take around $0.24\delta$ microseconds for each of its two invocations, and $\mathsf{VPSS}_{1}.\mathsf{Verify}$ to take around $7\ell|C|$ microseconds. For $t\leq 4$, the latter dominates, for $t=5$, they are roughly comparable, and for $t\geq 6$, the former quickly dominates.

For comparison, we also show the computational time for MuSig-DN, but *only* the zero-knowledge proof and verification components of their algorithm. We ran their code *[45]* on our same machine to obtain these figures. We can see that for $n\leq 20$, Arctic is more than an order of magnitude faster than MuSig-DN, and for $n\leq 10$, it is three orders of magnitude faster.

As seen in Figure 7(a), the computation time for the $(n,|C|,t)=(25,25,11)$ parameter combination, where $\delta={24\choose 10}=1961256$ is around $940\,\mathrm{ms}$, almost all of which is spent in $\mathsf{VPSS}_{1}.\mathsf{Gen}$ computing Equation 4. However, we observe that Equation 4 computes the sum of $\delta$ independent terms, and so is highly amenable to parallelization, which we also implemented and measured. We ran this scaling experiment both on the above machine, and also on a 40-core 2.3 GHz Intel 8380 CPU. The results are shown in Figure 7(b). Although the slower clock speed of the 40-core CPU puts it at a disadvantage for smaller numbers of cores, we can see almost linear scaling for both CPUs. Using all 4 cores, the 4-core CPU sees a speedup of $3.69\times$ for $\mathsf{VPSS}_{1}.\mathsf{Gen}$ and $3.65\times$ in total time, while using 32 cores, the 40-core CPU sees a speedup of $27.9\times$ for $\mathsf{VPSS}_{1}.\mathsf{Gen}$ and $25.7\times$ in total time. Beyond 32 cores, we observed diminishing returns.

## 7 Conclusion

In this work, we presented Arctic, a deterministic and stateless threshold Schnorr signature scheme for the honest majority setting. By not requiring zero-knowledge proofs of verifiable random functions, Arctic is simpler than previous deterministic threshold Schnorr schemes, and for small to moderate sized groups of signers, Arctic is one to three orders of magnitude faster.

## Acknowledgements

We thank Douglas Stebila for discussion and his feedback on this work. We thank Lior Eagen for his discussion and help estimating the computational overhead of Musig-DN proofs. This work benefited from the use of the CrySP RIPPLE Facility at the University of Waterloo. This research was undertaken, in part, thanks to funding from the Canada Research Chairs program, award CRC-2018-00135.

## References

- [1] M. Bellare, E. C. Crites, C. Komlo, M. Maller, S. Tessaro, and C. Zhu. Better than Advertised Security for Non-interactive Threshold Signatures. In Y. Dodis and T. Shrimpton, editors, CRYPTO 2022, volume 13510 of LNCS, pages 517–550. Springer, 2022.
-

2] M. Bellare and G. Neven. Multi-Signatures in the Plain Public-Key Model and a General Forking Lemma. In A. Juels, R. N. Wright, and S. D. C. di Vimercati, editors, Proceedings of the 13th ACM Conference on Computer and Communications Security, CCS 2006, Alexandria, VA, USA, October 30 - November 3, 2006, pages 390–399. ACM, 2006.
- [3] M. Bellare and P. Rogaway. The Security of Triple Encryption and a Framework for Code-Based Game-Playing Proofs. In S. Vaudenay, editor, EUROCRYPT 2006, 25th Annual International Conference on the Theory and Applications of Cryptographic Techniques, St. Petersburg, Russia, May 28 - June 1, 2006, volume 4004 of Lecture Notes in Computer Science, pages 409–426. Springer, 2006. doi:10.1007/11761679_25.
- [4] F. Benhamouda, E. Boyle, N. Gilboa, S. Halevi, Y. Ishai, and A. Nof. Generalized Pseudorandom Secret Sharing and Efficient Straggler-Resilient Secure Computation. In K. Nissim and B. Waters, editors, Theory of Cryptography - 19th International Conference, TCC 2021, Raleigh, NC, USA, November 8-11, 2021, volume 13043 of LNCS, pages 129–161. Springer, 2021. doi:10.1007/978-3-030-90453-1_5.
- [5] F. Benhamouda, T. Lepoint, J. Loss, M. Orrù, and M. Raykova. On the (in)security of ROS. In A. Canteaut and F. Standaert, editors, EUROCRYPT 2021, Zagreb, Croatia, October 17-21, 2021, volume 12696 of LNCS, pages 33–53. Springer, 2021.
- [6] A. Boldyreva. Threshold Signatures, Multisignatures and Blind Signatures Based on the Gap-Diffie-Hellman-Group Signature Scheme. In Y. Desmedt, editor, PKC 2003, Miami, FL, USA, January 6-8, 2003, volume 2567 of LNCS, pages 31–46. Springer, 2003.
- [7] D. Boneh, B. Lynn, and H. Shacham. Short Signatures from the Weil Pairing. J. Cryptol., 17(4):297–319, 2004.
- [8] C. Bonte, N. P. Smart, and T. Tanguy. Thresholdizing HashEdDSA: MPC to the Rescue. Int. J. Inf. Sec., 20(6):879–894, 2021. doi:10.1007/s10207-021-00539-6.
- [9] E. Boyle, G. Couteau, N. Gilboa, Y. Ishai, L. Kohl, and P. Scholl. Efficient Pseudorandom Correlation Generators from Ring-LPN. In D. Micciancio and T. Ristenpart, editors, CRYPTO 2020 - 40th Annual International Cryptology Conference, CRYPTO 2020, Santa Barbara, CA, USA, August 17-21, 2020, volume 12171 of Lecture Notes in Computer Science, pages 387–416. Springer, 2020. doi:10.1007/978-3-030-56880-1_14.
- [10] B. Bünz, J. Bootle, D. Boneh, A. Poelstra, P. Wuille, and G. Maxwell. Bulletproofs: Short Proofs for Confidential Transactions and More. In 2018 IEEE Symposium on Security and Privacy, SP 2018, Proceedings, 21-23 May 2018, San Francisco, California, USA, pages 315–334. IEEE Computer Society, 2018. doi:10.1109/SP.2018.00020.
- [11] R. Canetti, R. Gennaro, S. Goldfeder, N. Makriyannis, and U. Peled. UC non-interactive, proactive, threshold ECDSA with identifiable aborts. In J. Ligatti, X. Ou, J. Katz, and G. Vigna, editors, CCS ’20: 2020 ACM SIGSAC Conference on Computer and Communications Security, Virtual Event, USA, November 9-13, 2020, pages 1769–1787. ACM, 2020.
- [12] I. Cascudo and B. David. SCRAPE: Scalable Randomness Attested by Public Entities. In D. Gollmann, A. Miyaji, and H. Kikuchi, editors, Applied Cryptography and Network Security (ACNS) 2017, volume 10355 of LNCS, pages 537–556. Springer, 2017.
- [13] H. Chu, P. Gerhart, T. Ruffing, and D. Schröder. Practical Schnorr Threshold Signatures Without the Algebraic Group Model. In H. Handschuh and A. Lysyanskaya, editors, CRYPTO 2023 - 43rd Annual International Cryptology Conference,

CRYPTO 2023, Santa Barbara, CA, USA, August 20-24, 2023, volume 14081 of LNCS, pages 743–773. Springer, 2023. doi:10.1007/978-3-031-38557-5_24.
- [14] D. Connolly, C. Komlo, I. Goldberg, and C. Wood. The Flexible Round-Optimized Schnorr Threshold (FROST) Protocol for Two-Round Schnorr Signatures, 2024. URL: https://www.rfc-editor.org/rfc/rfc9591.html.
- [15] R. Cramer, I. Damgård, and Y. Ishai. Share Conversion, Pseudorandom Secret-Sharing and Applications to Secure Computation. In J. Kilian, editor, Theory of Cryptography, Second Theory of Cryptography Conference, TCC 2005, Cambridge, MA, USA, February 10-12, 2005, Proceedings, volume 3378 of Lecture Notes in Computer Science, pages 342–362. Springer, 2005. doi:10.1007/978-3-540-30576-7_19.
- [16] E. C. Crites, C. Komlo, and M. Maller. Fully Adaptive Schnorr Threshold Signatures. In H. Handschuh and A. Lysyanskaya, editors, CRYPTO 2023 - 43rd Annual International Cryptology Conference, CRYPTO 2023, Santa Barbara, CA, USA, August 20-24, 2023, volume 14081 of LNCS, pages 678–709. Springer, 2023. doi:10.1007/978-3-031-38557-5_22.
- [17] A. P. K. Dalskov, C. Orlandi, M. Keller, K. Shrishak, and H. Schulmann. Securing DNSSEC Keys via Threshold ECDSA from Generic MPC. In L. Chen, N. Li, K. Liang, and S. A. Schneider, editors, ESORICS 2020 - 25th European Symposium on Research in Computer Security, ESORICS 2020, Guildford, UK, September 14-18, 2020, volume 12309 of LNCS, pages 654–673. Springer, 2020. doi:10.1007/978-3-030-59013-0_32.
- [18] I. Damgård, T. P. Jakobsen, J. B. Nielsen, J. I. Pagter, and M. B. Østergaard. Fast threshold ECDSA with honest majority. J. Comput. Secur., 30(1):167–196, 2022. doi:10.3233/JCS-200112.
- [19] Y. Dodis. Efficient Construction of (Distributed) Verifiable Random Functions. In Y. Desmedt, editor, Public Key Cryptography - PKC 2003, 6th International Workshop on Theory and Practice in Public Key Cryptography, Miami, FL, USA, January 6-8, 2003, Proceedings, volume 2567 of Lecture Notes in Computer Science, pages 1–17. Springer, 2003. doi:10.1007/3-540-36288-6_1.
- [20] J. Doerner, Y. Kondi, E. Lee, and A. Shelat. Threshold ECDSA in Three Rounds. URL: https://eprint.iacr.org/2023/765.
- [21] M. Drijvers, K. Edalatnejad, B. Ford, E. Kiltz, J. Loss, G. Neven, and I. Stepanovs. On the Security of Two-Round Multi-Signatures. In SP 2019, San Francisco, CA, USA, May 19-23, 2019, pages 1084–1101. IEEE, 2019.
- [22] A. Fiat and A. Shamir. How to Prove Yourself: Practical Solutions to Identification and Signature Problems. In A. M. Odlyzko, editor, CRYPTO 1986, Santa Barbara, California, USA, 1986, volume 263 of LNCS, pages 186–194. Springer, 1986.
- [23] M. Fischlin. Communication-Efficient Non-interactive Proofs of Knowledge with Online Extractors. In V. Shoup, editor, CRYPTO 2005, volume 3621 of LNCS, pages 152–168. Springer, 2005.
- [24] D. Galindo, J. Liu, M. Ordean, and J. Wong. Fully distributed verifiable random functions and their application to decentralised random beacons. In IEEE European Symposium on Security and Privacy, EuroSSP 2021, Vienna, Austria, September 6-10, 2021, pages 88–102. IEEE, 2021. URL: https://doi.org/10.1109/EuroSP51992.2021.00017, doi:10.1109/EUROSP51992.2021.00017.
- [25] F. Garillot, Y. Kondi, P. Mohassel, and V. Nikolaenko. Threshold Schnorr with Stateless Deterministic Signing from Standard Assumptions. In T. Malkin and C. Peikert, editors, CRYPTO 2021 - 41st Annual International Cryptology Conference, CRYPTO 2021, Virtual Event, August 16-20, 2021, volume 12825 of LNCS, pages 127–156. Springer, 2021. doi:10.1007/978-3-030-84242-0_6.

26. R. Gennaro and S. Goldfeder. Fast Multiparty Threshold ECDSA with Fast Trustless Setup. In D. Lie, M. Mannan, M. Backes, and X. Wang, editors, CCS 2018, Toronto, ON, Canada, October 15-19, 2018, pages 1179–1194. ACM, 2018.
- 27. R. Gennaro, S. Jarecki, H. Krawczyk, and T. Rabin. Robust Threshold DSS Signatures. In U. M. Maurer, editor, Advances in Cryptology - EUROCRYPT ’96, International Conference on the Theory and Application of Cryptographic Techniques, Saragossa, Spain, May 12-16, 1996, Proceeding, volume 1070 of Lecture Notes in Computer Science, pages 354–371. Springer, 1996. doi:10.1007/3-540-68339-9_31.
- 28. R. Gennaro, S. Jarecki, H. Krawczyk, and T. Rabin. Robust Threshold DSS Signatures. Inf. Comput., 164(1):54–84, 2001.
- 29. R. Gennaro, S. Jarecki, H. Krawczyk, and T. Rabin. Secure Distributed Key Generation for Discrete-Log Based Cryptosystems. J. Cryptol., 20(1):51–83, 2007.
- 30. R. Gennaro, T. Rabin, S. Jarecki, and H. Krawczyk. Robust and Efficient Sharing of RSA Functions. J. Cryptol., 20(3):393, 2007.
- 31. S. Goldwasser, S. Micali, and R. L. Rivest. A Digital Signature Scheme Secure Against Adaptive Chosen-Message Attacks. SIAM J. Comput., 17(2):281–308, 1988. doi:10.1137/0217017.
- 32. M. Ito, A. Saito, and T. Nishizeki. Secret sharing scheme realizing general access structure. Electronics and Communications in Japan (Part III: Fundamental Electronic Science), 72(9):56–64, 1989. URL: https://onlinelibrary.wiley.com/doi/abs/10.1002/ecjc.4430720906, arXiv:https://onlinelibrary.wiley.com/doi/pdf/10.1002/ecjc.4430720906, doi:https://doi.org/10.1002/ecjc.4430720906.
- 33. S. Jarecki, H. Krawczyk, and J. Resch. Threshold partially-oblivious PRFs with applications to key management. 2018. URL: https://eprint.iacr.org/2018/733.
- 34. M. Jawurek, F. Kerschbaum, and C. Orlandi. Zero-knowledge using garbled circuits: how to prove non-algebraic statements efficiently. In A. Sadeghi, V. D. Gligor, and M. Yung, editors, ACM SIGSAC Conference on Computer and Communications Security, CCS’13, Berlin, Germany, November 4-8, 2013, pages 955–966. ACM, 2013. doi:10.1145/2508859.2516662.
- 35. J. Katz. Round Optimal Robust Distributed Key Generation. 2023. URL: https://eprint.iacr.org/2023/1094.
- 36. C. Komlo and I. Goldberg. FROST: Flexible Round-Optimized Schnorr Threshold Signatures. In O. Dunkelman, M. J. J. Jr., and C. O’Flynn, editors, Selected Areas in Cryptography - SAC 2020, volume 12804 of LNCS, pages 34–65. Springer, 2020. doi:10.1007/978-3-030-81652-0_2.
- 37. C. Komlo and I. Goldberg. Arctic: Lightweight and Stateless Threshold Schnorr Signatures. Cryptology ePrint Archive, Paper 2024/466, 2025. URL: https://eprint.iacr.org/2024/466.
- 38. Y. Kondi. Personal Communication, 2024.
- 39. Y. Kondi, C. Orlandi, and L. Roy. Two-Round Stateless Deterministic Two-Party Schnorr Signatures from Pseudorandom Correlation Functions. In H. Handschuh and A. Lysyanskaya, editors, CRYPTO 2023 - 43rd Annual International Cryptology Conference, CRYPTO 2023, Santa Barbara, CA, USA, August 20-24, 2023, volume 14081 of Lecture Notes in Computer Science, pages 646–677. Springer, 2023. doi:10.1007/978-3-031-38557-5_21.
- 40. Y. Lindell. Simple Three-Round Multiparty Schnorr Signing with Full Simulatability. Cryptology ePrint Archive, Report 2022/374, 2022. https://ia.cr/2022/374.
- 41. N. Makriyannis. On the Classic Protocol for MPC Schnorr Signatures. Cryptology ePrint Archive, Paper 2022/1332, 2022. https://eprint.iacr.org/2022/1332. URL: https://eprint.iacr.org/2022/1332.

42. N. Makriyannis, O. Yomtov, and A. Galansky. Practical Key-Extraction Attacks in Leading MPC Wallets. Cryptology ePrint Archive, Paper 2023/1234, 2023. URL: https://eprint.iacr.org/2023/1234.
- 43. S. Micali, M. O. Rabin, and S. P. Vadhan. Verifiable Random Functions. In 40th Annual Symposium on Foundations of Computer Science, FOCS ’99, 17-18 October, 1999, New York, NY, USA, pages 120–130. IEEE Computer Society, 1999. doi:10.1109/SFFCS.1999.814584.
- 44. M. Naor, B. Pinkas, and O. Reingold. Distributed Pseudo-random Functions and KDCs. In J. Stern, editor, EUROCRYPT ’99, volume 1592 of LNCS, pages 327–346. Springer, 1999.
- 45. J. Nick. Switch bulletproof example to purify. https://github.com/jonasnick/secp256k1-zkp/blob/bulletproof-musig-dn-benches/examples/bulletproof.c, 2020.
- 46. J. Nick, T. Ruffing, Y. Seurin, and P. Wuille. MuSig-DN: Schnorr Multi-Signatures with Verifiably Deterministic Nonces. In J. Ligatti, X. Ou, J. Katz, and G. Vigna, editors, CCS ’20: 2020 ACM SIGSAC Conference on Computer and Communications Security, Virtual Event, USA, November 9-13, 2020, pages 1717–1731. ACM, 2020. doi:10.1145/3372297.3417236.
- 47. C. Orlandi, P. Scholl, and S. Yakoubov. The Rise of Paillier: Homomorphic Secret Sharing and Public-Key Silent OT. In A. Canteaut and F. Standaert, editors, EUROCRYPT 2021 - 40th Annual International Conference on the Theory and Applications of Cryptographic Techniques, Zagreb, Croatia, October 17-21, 2021, Proceedings, volume 12696 of LNCS, pages 678–708. Springer, 2021. doi:10.1007/978-3-030-77870-5_24.
- 48. D. Pointcheval and J. Stern. Security Proofs for Signature Schemes. In U. M. Maurer, editor, Advances in Cryptology - EUROCRYPT ’96, International Conference on the Theory and Application of Cryptographic Techniques, Saragossa, Spain, May 12-16, 1996, Proceeding, volume 1070 of Lecture Notes in Computer Science, pages 387–398. Springer, 1996. doi:10.1007/3-540-68339-9_33.
- 49. D. Pointcheval and J. Stern. Security Arguments for Digital Signatures and Blind Signatures. J. Cryptol., 13(3):361–396, 2000.
- 50. T. Ristenpart and S. Yilek. When Good Randomness Goes Bad: Virtual Machine Reset Vulnerabilities and Hedging Deployed Cryptography. In Proceedings of the Network and Distributed System Security Symposium, NDSS 2010, San Diego, California, USA, 28th February - 3rd March 2010. The Internet Society, 2010. URL: https://www.ndss-symposium.org/ndss2010/when-good-randomness-goes-bad-virtual-machine-reset-vulnerabilities-and-hedging-deployed.
- 51. T. Ruffing, V. Ronge, E. Jin, J. Schneider-Bensch, and D. Schröder. ROAST: Robust Asynchronous Schnorr Threshold Signatures. In H. Yin, A. Stavrou, C. Cremers, and E. Shi, editors, Proceedings of the 2022 ACM SIGSAC Conference on Computer and Communications Security, CCS 2022, Los Angeles, CA, USA, November 7-11, 2022, pages 2551–2564. ACM, 2022. doi:10.1145/3548606.3560583.
- 52. C. Schnorr. Efficient Signature Generation by Smart Cards. J. Cryptol., 4(3):161–174, 1991.
- 53. C. Schnorr. Enhancing the security of perfect blind DL-signatures. Inf. Sci., 176(10):1305–1320, 2006. URL: https://doi.org/10.1016/j.ins.2005.04.007, doi:10.1016/J.INS.2005.04.007.
- 54. A. Shamir. How to Share a Secret. Commun. ACM, 22(11):612–613, 1979.
- 55. D. R. Stinson and R. Strobl. Provably Secure Distributed Schnorr Signatures and a (t, n) Threshold Scheme for Implicit Certificates. In V. Varadharajan and Y. Mu, editors, ACISP 2001, Sydney, Australia, July 11-13, 2001, volume 2119 of LNCS, pages 417–434. Springer, 2001.

56] H. W. H. Wong, J. P. K. Ma, H. H. F. Yin, and S. S. M. Chow. How (Not) to Build Threshold EdDSA. In Proceedings of the 26th International Symposium on Research in Attacks, Intrusions and Defenses, RAID 2023, Hong Kong, China, October 16-18, 2023, pages 123–134. ACM, 2023. doi:10.1145/3607199.3607230.