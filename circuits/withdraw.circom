pragma circom 2.2.3;

// The withdraw statement, specified in docs/SPEC.md section 5.
//
// I know (k, r, denomTag) such that the leaf H3(k, r, denomTag) sits at the
// same index in the pool tree and in the clean tree, my nullifier is H1(k),
// and this proof is bound to `binding`.

include "circomlib/circuits/poseidon.circom";

// Folds `leaf` up a depth-`depth` Poseidon tree. Bit j of the index is
// pathIndices[j]: 0 puts the running node on the left, 1 on the right. This is
// mirror-core's MerkleProof::fold. The bits are constrained by the caller,
// because two trees share them.
template MerkleRoot(depth) {
    signal input leaf;
    signal input pathIndices[depth];
    signal input siblings[depth];
    signal output root;

    signal node[depth + 1];
    signal left[depth];
    signal right[depth];
    component hashers[depth];

    node[0] <== leaf;
    for (var j = 0; j < depth; j++) {
        // bit 0: (node, sibling). bit 1: (sibling, node).
        left[j] <== node[j] + pathIndices[j] * (siblings[j] - node[j]);
        right[j] <== siblings[j] + pathIndices[j] * (node[j] - siblings[j]);
        hashers[j] = Poseidon(2);
        hashers[j].inputs[0] <== left[j];
        hashers[j].inputs[1] <== right[j];
        node[j + 1] <== hashers[j].out;
    }
    root <== node[depth];
}

template Withdraw(depth) {
    // Public inputs. circom orders them by declaration, so this order is the
    // verifier's order: poolRoot, cleanRoot, nullifierHash, binding.
    signal input poolRoot;
    signal input cleanRoot;
    signal input nullifierHash;
    signal input binding;

    // Private inputs.
    signal input k;
    signal input r;
    signal input denomTag;
    signal input pathIndices[depth];
    signal input poolSiblings[depth];
    signal input cleanSiblings[depth];

    component leafHasher = Poseidon(3);
    leafHasher.inputs[0] <== k;
    leafHasher.inputs[1] <== r;
    leafHasher.inputs[2] <== denomTag;

    component nullifierHasher = Poseidon(1);
    nullifierHasher.inputs[0] <== k;
    nullifierHash === nullifierHasher.out;

    // One index for both trees: the clean tree is a mask over the pool tree.
    for (var j = 0; j < depth; j++) {
        pathIndices[j] * (pathIndices[j] - 1) === 0;
    }

    component poolTree = MerkleRoot(depth);
    poolTree.leaf <== leafHasher.out;
    poolTree.pathIndices <== pathIndices;
    poolTree.siblings <== poolSiblings;
    poolRoot === poolTree.root;

    component cleanTree = MerkleRoot(depth);
    cleanTree.leaf <== leafHasher.out;
    cleanTree.pathIndices <== pathIndices;
    cleanTree.siblings <== cleanSiblings;
    cleanRoot === cleanTree.root;

    // The binding takes part in a constraint, so the compiler cannot drop it
    // and a proof cannot be replayed under a different action.
    signal bindingSquare;
    bindingSquare <== binding * binding;
}

component main {public [poolRoot, cleanRoot, nullifierHash, binding]} = Withdraw(20);
