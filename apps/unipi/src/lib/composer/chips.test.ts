import { describe, expect, it } from 'vitest';
import { attachmentToken, chipSpans, fileChipIds, imageChipIds, imageToken, insertToken, pathCandidates, removeToken, resolveAttachmentTokens, resolveImageTokens, restoredImageText, serializeSkills, skillRefs, stripAttachmentTokens, tokenize } from './chips';
import { appendFileMentions, fileMentionsOf, formatFileMention } from './attachments';

const cmds = new Set(['skill:review', 'unipi:goal']);

describe('tokenize (UNI-212)', () => {
  it('chips attachment tokens wherever they sit, plain text around them', () => {
    const text = `Hello world, this is the file: ${attachmentToken('a.pdf', 'f1')} and more`;
    expect(tokenize(text)).toEqual([
      { text: 'Hello world, this is the file: ' },
      { chip: { kind: 'file', raw: '[file: a.pdf]@att:f1', name: 'a.pdf', attId: 'f1' } },
      { text: ' and more' },
    ]);
  });

  it('a host-path mention (the wire form) chips too', () => {
    const segs = tokenize('see [file: x.txt]@/home/u/.unipi/app-host/blobs/ab12');
    expect(segs[1]).toEqual({ chip: { kind: 'file', raw: '[file: x.txt]@/home/u/.unipi/app-host/blobs/ab12', name: 'x.txt', path: '/home/u/.unipi/app-host/blobs/ab12' } });
  });

  it('a leading /skill: or /command chips only when it is in the session list and finished', () => {
    expect(tokenize('/skill:review the diff', { commands: cmds })[0]).toEqual({ chip: { kind: 'skill', raw: '/skill:review', name: 'review' } });
    expect(tokenize('/unipi:goal ship', { commands: cmds })[0]).toEqual({ chip: { kind: 'command', raw: '/unipi:goal', name: 'unipi:goal', mode: true } });
    expect(tokenize('/skill:nope x', { commands: cmds })).toEqual([{ text: '/skill:nope x' }]);
    expect(tokenize('say /unipi:goal', { commands: cmds })).toEqual([{ text: 'say /unipi:goal' }]);
    // Editor mode: still being typed (no trailing space) → plain.
    expect(tokenize('/unipi:goal', { commands: cmds, requireTrailingSpace: true })).toEqual([{ text: '/unipi:goal' }]);
    expect(tokenize('/unipi:goal ', { commands: cmds, requireTrailingSpace: true })[0]).toHaveProperty('chip');
  });

  it('@paths chip only once verified', () => {
    const verified = (t: string) => t === '@src/a.ts';
    const segs = tokenize('look at @src/a.ts and @src/nope.ts ok', { verified });
    expect(segs.map((s) => ('chip' in s ? `[${s.chip.raw}]` : s.text)).join('')).toBe('look at [@src/a.ts] and @src/nope.ts ok');
  });

  it("collapses pi's expanded skill block in bubbles", () => {
    const text = '<skill name="review" location="/x/SKILL.md">\nReferences are relative to /x.\n\nbody\n</skill>\n\nthe diff please';
    expect(tokenize(text, { skillBlocks: true })).toEqual([{ chip: { kind: 'skill', raw: expect.any(String), name: 'review' } }, { text: 'the diff please' }]);
  });

  it('chipSpans gives each chip its text offset', () => {
    const text = `a ${attachmentToken('x', '1')} b ${attachmentToken('y', '2')}`;
    expect(chipSpans(tokenize(text)).map((s) => s.start)).toEqual([2, text.indexOf('[file: y]')]);
  });
});

describe('editing helpers', () => {
  it('insertToken pads with spaces only where needed', () => {
    expect(insertToken('hello', 5, 'T')).toEqual({ text: 'hello T ', caret: 8 });
    expect(insertToken('hello world', 6, 'T')).toEqual({ text: 'hello T world', caret: 8 });
    expect(insertToken('', 0, 'T')).toEqual({ text: 'T ', caret: 2 });
  });
  it('removeToken drops the token and one neighbouring space', () => {
    expect(removeToken('a T b', 2, 'T')).toEqual({ text: 'a b', caret: 2 });
    expect(removeToken('a T', 2, 'T')).toEqual({ text: 'a', caret: 1 });
    expect(removeToken('a X b', 2, 'T')).toEqual({ text: 'a X b', caret: 2 });
  });
  it('pathCandidates: finished @tokens only, never attachment refs', () => {
    expect(pathCandidates('see @src/a.ts and @"my file.md" now @typing')).toEqual(['@src/a.ts', '@"my file.md"']);
    expect(pathCandidates(`${attachmentToken('a', '1')} x`)).toEqual([]);
  });
});

describe('resolveAttachmentTokens: the wire format stays the UNI-203 mention', () => {
  const files = (id: string) =>
    ({
      f1: { name: 'a.pdf', status: 'done' as const, path: '/blobs/aaa' },
      f2: { name: 'b.txt', status: 'uploading' as const },
      f3: { name: 'c.png', status: 'error' as const },
    })[id];

  it('done → `[file: name]@<host path>` in place, exactly formatFileMention', () => {
    const r = resolveAttachmentTokens(`Hello, the file: ${attachmentToken('a.pdf', 'f1')} and more`, files);
    expect(r.text).toBe(`Hello, the file: ${formatFileMention({ name: 'a.pdf', path: '/blobs/aaa' })} and more`);
    expect(fileMentionsOf(r.text)).toEqual([expect.objectContaining({ name: 'a.pdf', path: '/blobs/aaa' })]);
    expect(r.sentIds).toEqual(['f1']);
  });

  it('a chip at the end is the same text the old appendFileMentions produced', () => {
    const r = resolveAttachmentTokens(`look\n\n${attachmentToken('a.pdf', 'f1')}`, files);
    expect(r.text).toBe(appendFileMentions('look', [{ id: 'f1', name: 'a.pdf', mime: 'x', size: 1, status: 'done', path: '/blobs/aaa' }]));
  });

  it('uploading/failed tokens are left out of the text and kept for the draft; unknown ones dropped', () => {
    const r = resolveAttachmentTokens(`a ${attachmentToken('b.txt', 'f2')} b ${attachmentToken('c.png', 'f3')} c ${attachmentToken('z', 'gone')}`, files);
    expect(r.text).toBe('a b c ');
    expect(r.kept).toEqual(['[file: b.txt]@att:f2', '[file: c.png]@att:f3']);
  });

  it('text with no tokens is untouched', () => {
    expect(resolveAttachmentTokens('plain @src/a.ts /unipi:goal', files).text).toBe('plain @src/a.ts /unipi:goal');
  });
});

describe('bubbleSegments (UNI-212 sent bubbles)', async () => {
  const { bubbleSegments } = await import('../../components/timeline/UserBubble');
  it('many attachment mentions collapse into one wrap of chips (no line per file)', () => {
    const text = 'look\n\n' + ['a', 'b', 'c'].map((n) => `[file: ${n}.txt]@/blobs/${n}`).join('\n');
    const segs = bubbleSegments(text, new Set());
    expect(segs.map((s) => ('chip' in s ? `<${s.chip.kind}:${(s.chip as { name: string }).name}>` : s.text)).join('')).toBe('look\n\n<file:a.txt> <file:b.txt> <file:c.txt>');
  });
  it('chips a leading skill / command and keeps the rest as text', () => {
    const segs = bubbleSegments('/unipi:goal ship it', new Set(['unipi:goal']));
    expect(segs[0]).toEqual({ chip: { kind: 'command', raw: '/unipi:goal', name: 'unipi:goal', mode: true } });
    expect(segs[1]).toEqual({ text: ' ship it' });
  });
});

describe('image chips (UNI-285)', () => {
  const img = (id: string) => imageToken(id);

  it('tokenizes a pending image draft token wherever it sits', () => {
    const text = `compare ${img('i1')} with ${img('i2')} please`;
    expect(tokenize(text)).toEqual([
      { text: 'compare ' },
      { chip: { kind: 'image', raw: '[image]@img:i1', imgId: 'i1' } },
      { text: ' with ' },
      { chip: { kind: 'image', raw: '[image]@img:i2', imgId: 'i2' } },
      { text: ' please' },
    ]);
  });

  it('a `[image N]` only chips in a sent bubble (imageLabels), never in the composer draft', () => {
    expect(tokenize('compare [image 1] with [image 2]')).toEqual([{ text: 'compare [image 1] with [image 2]' }]);
    expect(tokenize('compare [image 1] with [image 2]', { imageLabels: true })).toEqual([
      { text: 'compare ' },
      { chip: { kind: 'image', raw: '[image 1]', n: 1 } },
      { text: ' with ' },
      { chip: { kind: 'image', raw: '[image 2]', n: 2 } },
    ]);
    // `[image 9]` with only two images still chips (the number is the text's).
    expect(tokenize('[image 9]', { imageLabels: true })[0]).toEqual({ chip: { kind: 'image', raw: '[image 9]', n: 9 } });
  });

  it('an image token is never mistaken for an @path candidate', () => {
    expect(pathCandidates(`${img('i1')} x`)).toEqual([]);
    expect(pathCandidates(`${attachmentToken('a', '1')} ${img('i1')} @src/a.ts `)).toEqual(['@src/a.ts']);
  });

  it('resolveImageTokens numbers each chip by its position among the resolving ones, and returns them in that order', () => {
    const images = [{ id: 'i1' }, { id: 'i2' }];
    const r = resolveImageTokens(`compare ${img('i2')} with ${img('i1')}`, images);
    expect(r.text).toBe('compare [image 1] with [image 2]');
    // ...and the caller sends the images in exactly that order, so N always matches.
    expect(r.ids).toEqual(['i2', 'i1']);
  });

  it('imageChipIds is the one numbering source: chip position in the draft', () => {
    expect(imageChipIds(`a ${img('i1')} b ${img('i2')}`)).toEqual(['i1', 'i2']);
    expect(imageChipIds('nothing here')).toEqual([]);
  });

  it('a token whose image is gone is dropped, with its separating space', () => {
    const r = resolveImageTokens(`a ${img('gone')} b`, [{ id: 'i1' }]);
    expect(r.text).toBe('a b');
    expect(r.ids).toEqual([]);
  });

  it('chipSpans and removeToken work on image tokens like any other chip', () => {
    const text = `a ${img('i1')} b`;
    const spans = chipSpans(tokenize(text));
    expect(spans).toEqual([{ chip: { kind: 'image', raw: '[image]@img:i1', imgId: 'i1' }, start: 2 }]);
    expect(removeToken(text, 2, '[image]@img:i1')).toEqual({ text: 'a b', caret: 2 });
  });

  it('bubbleSegments champs `[image N]` back to a chip (UNI-285 sent bubble)', async () => {
    const { bubbleSegments } = await import('../../components/timeline/UserBubble');
    const segs = bubbleSegments('compare [image 1] with [image 2]', new Set());
    expect(segs.map((s) => ('chip' in s ? `image ${(s.chip as { n: number }).n}` : s.text)).join('|')).toBe('compare |image 1| with |image 2');
  });
});

describe('restoredImageText (UNI-290: a sent message back into the draft)', () => {
  it('each [image N] becomes image N’s chip token; text without images is unchanged', () => {
    expect(restoredImageText('compare [image 1] with [image 2]', ['a', 'b'])).toBe(`compare ${imageToken('a')} with ${imageToken('b')}`);
    expect(restoredImageText('plain text', [])).toBe('plain text');
  });

  it('an image the text never mentioned gets its chip at the end; a repeated mention stays plain text', () => {
    expect(restoredImageText('look', ['a'])).toBe(`look ${imageToken('a')} `);
    expect(restoredImageText('[image 1] and again [image 1]', ['a'])).toBe(`${imageToken('a')} and again [image 1]`);
    // [image 3] with only two images: left alone.
    expect(restoredImageText('[image 3]', ['a', 'b'])).toBe(`[image 3] ${imageToken('a')} ${imageToken('b')} `);
  });

  it('round-trips with resolveImageTokens: what goes out again says the same thing', () => {
    const text = restoredImageText('compare [image 2] with [image 1]', ['a', 'b']);
    const sent = resolveImageTokens(text, [{ id: 'a' }, { id: 'b' }]);
    expect(sent.text).toBe('compare [image 1] with [image 2]');
    expect(sent.ids).toEqual(['b', 'a']);
  });
});

describe('composer chips (owner spec 2026-11-03)', () => {
  const skills = new Set(['skill:review', 'skill:pdf-tools', 'graph', 'unipi:goal', 'regular']);

  it('a $skill chips anywhere, only for a session skill and once finished', () => {
    const segs = tokenize('fix it with $review now', { commands: skills, requireTrailingSpace: true });
    expect(segs).toEqual([{ text: 'fix it with ' }, { chip: { kind: 'skill', raw: '$review', name: 'review', form: 'dollar' } }, { text: ' now' }]);
    expect(tokenize('fix it with $review', { commands: skills, requireTrailingSpace: true })).toEqual([{ text: 'fix it with $review' }]);
    expect(tokenize('costs $5 and $nope ', { commands: skills, requireTrailingSpace: true })).toEqual([{ text: 'costs $5 and $nope ' }]);
    expect(tokenize('a$review b', { commands: skills })).toEqual([{ text: 'a$review b' }]);
    // A bubble chips it before punctuation too.
    expect(tokenize('use $pdf-tools.', { commands: skills })[1]).toEqual({ chip: { kind: 'skill', raw: '$pdf-tools', name: 'pdf-tools', form: 'dollar' } });
  });

  it('several skill chips and a leading command chip coexist', () => {
    const segs = tokenize('/graph ship $review and $pdf-tools ', { commands: skills, requireTrailingSpace: true });
    expect(segs.filter((s) => 'chip' in s).map((s) => ('chip' in s ? s.chip.raw : ''))).toEqual(['/graph', '$review', '$pdf-tools']);
  });

  it('mode commands are command chips flagged mode', () => {
    expect(tokenize('/graph build it', { commands: skills })[0]).toEqual({ chip: { kind: 'command', raw: '/graph', name: 'graph', mode: true } });
    expect(tokenize('/unipi:goal x', { commands: skills })[0]).toEqual({ chip: { kind: 'command', raw: '/unipi:goal', name: 'unipi:goal', mode: true } });
    expect(tokenize('/regular', { commands: skills })[0]).toEqual({ chip: { kind: 'command', raw: '/regular', name: 'regular', mode: true } });
  });

  it('serializeSkills: the first $skill becomes the leading /skill: invocation pi expands', () => {
    expect(serializeSkills('$review fix the bug', skills)).toBe('/skill:review fix the bug');
    expect(serializeSkills('$review', skills)).toBe('/skill:review');
    expect(serializeSkills('$review\nline two', skills)).toBe('/skill:review line two');
    expect(serializeSkills('fix it with $review please', skills)).toBe('/skill:review fix it with $review please');
    expect(serializeSkills('$review then $pdf-tools', skills)).toBe('/skill:review then $pdf-tools');
    // A command keeps its arguments as typed; unknown skills and plain text are untouched.
    expect(serializeSkills('/graph do it with $review', skills)).toBe('/graph do it with $review');
    expect(serializeSkills('pay $5 to $nope', skills)).toBe('pay $5 to $nope');
    expect(skillRefs('a $review, b $pdf-tools', skills).map((r) => r.name)).toEqual(['review', 'pdf-tools']);
  });

  it('the same image chip twice (copy → paste) sends the image once, both mentions name it', () => {
    const t = `${imageToken('a')} vs ${imageToken('b')} and again ${imageToken('a')}`;
    expect(resolveImageTokens(t, [{ id: 'a' }, { id: 'b' }])).toEqual({ text: '[image 1] vs [image 2] and again [image 1]', ids: ['a', 'b'] });
  });

  it('a reloaded draft drops the tokens of attachments that lived in memory', () => {
    expect(stripAttachmentTokens(`look ${imageToken('x1')} at ${attachmentToken('a.pdf', 'f1')} this`)).toBe('look at this');
    expect(stripAttachmentTokens('keep [file: a.pdf]@/host/blobs/1 and [image 2]')).toBe('keep [file: a.pdf]@/host/blobs/1 and [image 2]');
    expect(fileChipIds(`${attachmentToken('a', 'f1')} [file: b]@/p ${attachmentToken('c', 'f2')}`)).toEqual(['f1', 'f2']);
  });
});
