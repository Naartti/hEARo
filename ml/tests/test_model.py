"""The detector is causal, streams with carried state like batch inference, and the teacher isn't."""

from __future__ import annotations

import numpy as np
import pytest
import torch

from hearo_ml.model import Detector, DetectorConfig


def model(**kw) -> Detector:
    torch.manual_seed(0)
    m = Detector(DetectorConfig(n_mels=8, channels=6, hidden=5, **kw))
    m.set_normalisation(np.full(8, -40.0), np.full(8, 10.0))
    return m.eval()


def feats(t: int = 50, seed: int = 0) -> torch.Tensor:
    return torch.as_tensor(np.random.default_rng(seed).normal(-40, 10, (1, t, 8)), dtype=torch.float32)


def test_outputs_never_depend_on_later_frames():
    m = model()
    a = feats()
    b = a.clone()
    b[:, 30:] = 0.0
    with torch.no_grad():
        ya, yb = m(a), m(b)
    assert torch.equal(ya[:, :30], yb[:, :30])
    assert not torch.equal(ya[:, 30:], yb[:, 30:])


def test_streaming_matches_batch_within_1e4():
    m = model(lookahead=2)
    x = feats(80, seed=1)
    with torch.no_grad():
        batch = m(x)[0]
        state, stream = m.initial_state(), []
        for t in range(x.shape[1]):
            y, state = m.step(x[0, t], state)
            stream.append(float(y))
    assert np.max(np.abs(np.array(stream) - batch.numpy())) <= 1e-4


def test_predict_realigns_by_the_lookahead():
    m = model(lookahead=3)
    x = feats(40)[0].numpy()
    p = m.predict(x)
    assert p.shape == (40,)
    with torch.no_grad():
        raw = torch.sigmoid(m(torch.as_tensor(x)[None]))[0].numpy()
    assert np.allclose(p[:30], raw[3:33])
    padded = np.concatenate([x, np.full((3, 8), -100.0, dtype=np.float32)])
    with torch.no_grad():
        tail = torch.sigmoid(m(torch.as_tensor(padded)[None]))[0].numpy()
    assert np.allclose(p, tail[3:]), "the last verdicts see the training crops' padding"


def test_teacher_sees_the_future_and_refuses_to_stream():
    m = model(bidirectional=True)
    a = feats()
    b = a.clone()
    b[:, 45:] = 0.0
    with torch.no_grad():
        assert not torch.equal(m(a)[:, :10], m(b)[:, :10])
    with pytest.raises(AssertionError):
        m.step(a[0, 0], m.initial_state())


def test_dropout_only_acts_while_training():
    m = model(dropout=0.5)
    x = feats(30)
    with torch.no_grad():
        assert torch.equal(m(x), m(x))
        m.train()
        assert not torch.equal(m(x), m(x))


def test_checkpoint_round_trip():
    m = model(lookahead=1)
    m2 = Detector.from_checkpoint(m.to_checkpoint())
    x = feats(20)[0].numpy()
    assert np.array_equal(m.predict(x), m2.predict(x))
    assert m.n_params() < 1_000_000
