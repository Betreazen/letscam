"""LetsCam's 3D scene for the project site (site.toml), built from code in Blender 5.2.

    "C:\\Program Files\\Blender Foundation\\Blender 5.2\\blender.exe" --background --factory-startup \
        --python site/scene/build_scene.py [-- --preview <folder>]

Writes two binary glTF files the site's viewer (forge3d.js) shows as they are:

    assets/site/letscam.glb  kind "mob": a player walking in place in front of an open doorway while
                             LetsCam's camera drone circles them (`idle`, looped), plus clips the
                             site plays once on a button: `flyby` (a low pass in front of the
                             player), `top` (straight overhead) and `shoulder` (over the shoulder).
    assets/site/drone.glb    kind "machine": the drone alone; its rotors are drill_* nodes (the site
                             spins them about Y) and its REC lamp is lit_rec (the site pulses it).

The site's rules: one block is one unit, Y up and the front facing +Z in glTF (Blender -Y here, the
exporter turns Z up into Y up), flat normals, pixel textures, no lights; a material named glow_*
glows in its own colour. --preview renders a few frames of every clip with EEVEE to check by eye.
"""
import math
import os
import random
import sys

import bmesh
import bpy
from mathutils import Matrix, Quaternion, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..', '..'))
OUT = os.path.join(ROOT, 'assets', 'site')
FPS = 30
PX = 1 / 16  # one texture pixel of a block, in blocks
SIZE_LIMIT = 4 * 1024 * 1024  # bytes per model (site.toml)

rand = random.Random(20261115)  # the same textures on every build


# ---------------------------------------------------------------------------- materials

def pixel_image(name, size, pixel):
    """A size[0] x size[1] image whose pixels come from pixel(x, y) -> (r, g, b)."""
    w, h = size
    img = bpy.data.images.new(name, w, h, alpha=False)
    data = []
    for y in range(h):
        for x in range(w):
            r, g, b = pixel(x, y)
            data += [r, g, b, 1.0]
    img.pixels = data
    img.pack()
    return img


def noisy(rgb, spread):
    """Minecraft-like block noise: every pixel a little lighter or darker than rgb."""
    def pixel(_x, _y):
        k = 1 + rand.uniform(-spread, spread)
        return tuple(min(1.0, max(0.0, c * k)) for c in rgb)
    return pixel


MATS = {}


def material(name, pixel=None, rgb=(1, 1, 1), size=(16, 16), spread=0.0):
    """A matte material with a NEAREST-filtered pixel texture (flat colour when spread is 0)."""
    if name in MATS:
        return MATS[name]
    mat = bpy.data.materials.new(name)
    tree = mat.node_tree
    bsdf = tree.nodes['Principled BSDF']
    bsdf.inputs['Roughness'].default_value = 1.0
    if pixel is None and spread == 0:
        bsdf.inputs['Base Color'].default_value = (*rgb, 1.0)
    else:
        img = pixel_image(name, size, pixel or noisy(rgb, spread))
        tex = tree.nodes.new('ShaderNodeTexImage')
        tex.image = img
        tex.interpolation = 'Closest'
        tree.links.new(tex.outputs['Color'], bsdf.inputs['Base Color'])
    MATS[name] = mat
    return mat


def grass_top(x, y):
    g = (0.36, 0.6, 0.22)
    k = 1 + rand.uniform(-0.14, 0.14)
    if rand.random() < 0.06:
        k = 1.25
    return tuple(c * k for c in g)


def planks(x, y):
    base = (0.66, 0.5, 0.3)
    k = 1 + rand.uniform(-0.06, 0.06)
    if y % 4 == 3:
        k = 0.72  # the seam between two boards
    if (x + (y // 4) * 7) % 16 == 0:
        k = 0.8  # the end of a board
    return tuple(c * k for c in base)


def stone(x, y):
    g = 0.5 + rand.uniform(-0.08, 0.08)
    if rand.random() < 0.08:
        g = 0.38
    return (g, g, g)


def clapper_stripes(x, y):
    return (0.92, 0.92, 0.9) if ((x + y) // 4) % 2 == 0 else (0.08, 0.08, 0.09)



# ---------------------------------------------------------------------------- geometry

def box(name, size, at=(0, 0, 0), mat=None, pivot=None, uv_block=True, parent=None):
    """An axis-aligned box of size (x, y, z) blocks centred on `at`, its origin at `pivot`
    (the box centre when None), flat-shaded; UVs repeat the texture once per block when uv_block,
    else stretch it over each face."""
    sx, sy, sz = size
    centre = Vector(at)
    origin = Vector(pivot) if pivot is not None else centre
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector((sx, sy, sz)), verts=bm.verts)
    bmesh.ops.translate(bm, vec=centre - origin, verts=bm.verts)
    uv = bm.loops.layers.uv.new()
    for face in bm.faces:
        n = face.normal
        for loop in face.loops:
            p = loop.vert.co + origin
            if abs(n.z) > 0.5:
                u, v = p.x, p.y
            elif abs(n.x) > 0.5:
                u, v = p.y, p.z
            else:
                u, v = p.x, p.z
            if not uv_block:
                u, v = 0.5 + 0.0 * u, 0.5 + 0.0 * v
            loop[uv].uv = (u, v)
    if not uv_block:
        stretch_uvs(bm, uv)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    for poly in mesh.polygons:
        poly.use_smooth = False
    ob = bpy.data.objects.new(name, mesh)
    ob.location = origin
    if mat is not None:
        mesh.materials.append(mat)
    bpy.context.scene.collection.objects.link(ob)
    if parent is not None:
        set_parent(ob, parent)
    return ob


def stretch_uvs(bm, uv):
    """Each face of a box gets the whole texture: (0,0)..(1,1) by the face's own corners."""
    for face in bm.faces:
        n = face.normal
        pts = [loop.vert.co for loop in face.loops]
        if abs(n.z) > 0.5:
            axes = (0, 1)
        elif abs(n.x) > 0.5:
            axes = (1, 2)
        else:
            axes = (0, 2)
        lo = [min(p[a] for p in pts) for a in axes]
        hi = [max(p[a] for p in pts) for a in axes]
        for loop in face.loops:
            p = loop.vert.co
            loop[uv].uv = tuple((p[a] - l) / ((h - l) or 1) for a, l, h in zip(axes, lo, hi))


def bar(name, a, b, thick, mat, parent=None):
    """A thin square bar from point a to point b (local to parent)."""
    a, b = Vector(a), Vector(b)
    length = (b - a).length
    ob = box(name, (thick, length, thick), (0, 0, 0), mat, uv_block=False)
    ob.rotation_mode = 'QUATERNION'
    ob.rotation_quaternion = (b - a).to_track_quat('Y', 'Z')
    ob.location = (a + b) / 2
    if parent is not None:
        set_parent(ob, parent)
    return ob


def empty(name, at=(0, 0, 0), parent=None):
    ob = bpy.data.objects.new(name, None)
    ob.location = at
    bpy.context.scene.collection.objects.link(ob)
    if parent is not None:
        set_parent(ob, parent)
    return ob


def set_parent(ob, parent):
    """Parents ob keeping its coordinates as given in the parent's space."""
    ob.parent = parent
    ob.matrix_parent_inverse = Matrix.Identity(4)


# ---------------------------------------------------------------------------- the scene

PLAYER_AT = Vector((0.0, -0.6, 0.0))
LOOK_AT = PLAYER_AT + Vector((0, 0, 1.45))  # between the player's chest and eyes


def build_ground():
    grass = material('grass', grass_top)
    dirt = material('dirt', rgb=(0.47, 0.33, 0.22), spread=0.12)
    path = material('stone_path', stone)
    root = empty('ground')
    box('grass', (7, 6, 0.5), (0, 0, -0.25 + 0.0), grass, parent=root)
    box('dirt', (7, 6, 0.5), (0, 0, -0.75), dirt, parent=root)
    for i, y in enumerate((-2.5, -1.5, -0.5)):
        box(f'path_{i}', (1, 1, 0.06), (0, y, 0.03), path, parent=root)
    flower_stem = material('stem', rgb=(0.2, 0.45, 0.15))
    flower = material('glow_flower', rgb=(0.95, 0.55, 0.15))
    for i, (x, y) in enumerate(((-2.6, -2.2), (2.4, -1.4), (-1.8, 1.9))):
        box(f'stem_{i}', (0.06, 0.06, 0.4), (x, y, 0.2), flower_stem, parent=root)
        box(f'bloom_{i}', (0.18, 0.18, 0.14), (x, y, 0.45), flower, parent=root)
    return root


def build_doorway():
    """An oak doorway the camera has to get through: two posts, a lintel and an open door."""
    wood = material('planks', planks)
    door = material('door', rgb=(0.55, 0.4, 0.22), spread=0.1)
    hinge = material('hinge', rgb=(0.3, 0.3, 0.32))
    root = empty('doorway', (0, 1.4, 0))
    box('post_left', (1, 1, 3), (-1, 0, 1.5), wood, parent=root)
    box('post_right', (1, 1, 3), (1, 0, 1.5), wood, parent=root)
    box('lintel', (3, 1, 1), (0, 0, 3.5), wood, parent=root)
    # The door stands open into the scene, hinged on the left post.
    leaf = box('door', (3 * PX, 1, 2), (0, 0.5, 1), door, pivot=(0, 0, 0), parent=root)
    leaf.location = (-0.5 + 1.5 * PX, -0.5, 0)
    leaf.rotation_euler = (0, 0, math.radians(170))
    box('door_hinge', (4 * PX, 3 * PX, 2 * PX), (0, 0.15, 1.6), hinge, parent=leaf)
    lamp = material('glow_lantern', rgb=(1.0, 0.72, 0.3))
    frame = material('lantern_frame', rgb=(0.18, 0.17, 0.16))
    box('lantern_cap', (0.36, 0.36, 0.08), (1, -0.62, 3.2), frame, parent=root)
    box('lantern', (0.26, 0.26, 0.3), (1, -0.62, 3.0), lamp, parent=root)
    return root


def build_player():
    """A blocky player in the game's proportions (2 blocks tall), the pivots where they bend."""
    skin = material('skin', rgb=(0.86, 0.66, 0.52), spread=0.03)
    hair = material('hair', rgb=(0.35, 0.2, 0.12), spread=0.08)
    shirt = material('shirt', rgb=(0.16, 0.5, 0.52), spread=0.06)
    pants = material('pants', rgb=(0.16, 0.18, 0.34), spread=0.06)
    shoes = material('shoes', rgb=(0.22, 0.2, 0.2), spread=0.04)
    eye = material('eye', rgb=(0.1, 0.12, 0.2))
    root = empty('player', PLAYER_AT)
    body = box('body', (0.5, 0.25, 0.75), (0, 0, 1.125), shirt, parent=root)
    head = box('head', (0.5, 0.5, 0.5), (0, 0, 1.75), skin, pivot=(0, 0, 1.5), parent=root)
    box('hair', (0.52, 0.52, 0.14), (0, 0.0, 0.45), hair, parent=head)
    box('hair_back', (0.52, 0.12, 0.4), (0, 0.2, 0.25), hair, parent=head)
    for i, x in enumerate((-0.11, 0.11)):
        box(f'eye_{i}', (0.08, 0.02, 0.06), (x, -0.26, 0.22), eye, parent=head)
    arms, legs = [], []
    for side, x in (('left', -0.375), ('right', 0.375)):
        arm = box(f'arm_{side}', (0.25, 0.25, 0.75), (x, 0, 1.125), shirt, pivot=(x, 0, 1.4), parent=root)
        box(f'hand_{side}', (0.252, 0.252, 0.2), (0, 0, -0.62), skin, parent=arm)
        leg = box(f'leg_{side}', (0.25, 0.25, 0.75), (x / 3, 0, 0.375), pants, pivot=(x / 3, 0, 0.75), parent=root)
        box(f'shoe_{side}', (0.254, 0.254, 0.14), (0, 0, -0.68), shoes, parent=leg)
        arms.append(arm)
        legs.append(leg)
    del body
    return root, head, arms, legs


def build_drone(name='drone', parent=None, rotor_prefix='rotor'):
    """LetsCam's camera: a body with a lens on +Y (its forward), four arms and rotors, a REC lamp
    and the frame of its view; the root's origin is the lens's optical centre."""
    shell = material('drone_shell', rgb=(0.2, 0.2, 0.23), spread=0.05)
    trim = material('drone_trim', rgb=(0.85, 0.48, 0.16), spread=0.04)
    dark = material('lens_glass', rgb=(0.04, 0.05, 0.07))
    ring = material('glow_lens', rgb=(1.0, 0.6, 0.2))
    rec = material('glow_rec', rgb=(1.0, 0.12, 0.08))
    blade = material('rotor_blade', rgb=(0.12, 0.12, 0.13))
    root = empty(name, parent=parent)
    box('drone_body', (0.56, 0.5, 0.3), (0, -0.25, 0), shell, parent=root)
    box('drone_stripe', (0.58, 0.3, 0.06), (0, -0.25, 0.12), trim, parent=root)
    box('lens_barrel', (0.26, 0.16, 0.26), (0, 0.05, 0), dark, parent=root)
    box('lens_ring', (0.3, 0.04, 0.3), (0, 0.15, 0), ring, parent=root)
    box('lens_glass', (0.18, 0.02, 0.18), (0, 0.175, 0), dark, parent=root)
    lamp = box('lit_rec' if name == 'drone_machine' else 'rec_lamp', (0.08, 0.08, 0.08), (0.2, -0.05, 0.17), rec, parent=root)
    rotors = []
    for i, (x, y) in enumerate(((-0.42, -0.6), (0.42, -0.6), (-0.42, 0.1), (0.42, 0.1))):
        bar(f'arm_{i}', (0, -0.25, 0.05), (x, y, 0.12), 0.06, shell, parent=root)
        hub = empty(f'{rotor_prefix}_{i}', (x, y, 0.18), parent=root)
        box(f'{rotor_prefix}_{i}_blade_a', (0.46, 0.06, 0.02), (0, 0, 0), blade, parent=hub)
        box(f'{rotor_prefix}_{i}_blade_b', (0.06, 0.46, 0.02), (0, 0, 0), blade, parent=hub)
        rotors.append(hub)
    return root, lamp, rotors


def build_frame(drone):
    """The drone's view: four amber lines from the lens out to a 16:9 frame, part of the drone."""
    glow = material('glow_frame', rgb=(1.0, 0.62, 0.22))
    near = Vector((0, 0.18, 0))
    far_y, hw, hh = 1.25, 0.42, 0.24
    corners = [Vector((sx * hw, far_y, sz * hh)) for sx, sz in ((-1, -1), (1, -1), (1, 1), (-1, 1))]
    for i, c in enumerate(corners):
        bar(f'view_ray_{i}', near, c, 0.022, glow, parent=drone)
        bar(f'view_edge_{i}', c, corners[(i + 1) % 4], 0.022, glow, parent=drone)


def build_props():
    """A clapperboard on the grass and a few leaf blocks round the set."""
    board = material('clapper_board', rgb=(0.1, 0.1, 0.11), spread=0.05)
    stripes = material('clapper_stripes', clapper_stripes, size=(16, 16))
    root = empty('props')
    clap = empty('clapper', (2.1, -2.1, 0.0), parent=root)
    clap.rotation_euler = (0, 0, math.radians(25))
    box('clapper_slate', (0.7, 0.08, 0.5), (0, 0, 0.3), board, parent=clap)
    box('clapper_base_stripes', (0.7, 0.1, 0.1), (0, 0, 0.6), stripes, uv_block=False, parent=clap)
    stick = empty('clapper_stick', (-0.35, 0, 0.66), parent=clap)
    box('clapper_stick_board', (0.7, 0.1, 0.1), (0.35, 0, 0.05), stripes, uv_block=False, parent=stick)
    leaves = material('leaves', rgb=(0.2, 0.42, 0.16), spread=0.22)
    box('leaves_block', (1, 1, 1), (-2.5, 2.0, 0.5), leaves, parent=root)
    box('leaves_bush', (0.6, 0.6, 0.5), (-2.3, 0.9, 0.25), leaves, parent=root)
    box('leaves_bush_2', (0.5, 0.5, 0.4), (2.6, 2.1, 0.2), leaves, parent=root)
    return root, stick


# ---------------------------------------------------------------------------- animation

def orbit_point(theta, radius, height, wobble=0.0):
    """A point of the ellipse round the player: radius = (sideways, front to back)."""
    rx, ry = radius
    return LOOK_AT + Vector((math.sin(theta) * rx, -math.cos(theta) * ry, height - LOOK_AT.z + wobble))


def aim(position, target):
    """The drone's rotation that points its +Y (the lens) at target, Z up."""
    return (Vector(target) - Vector(position)).to_track_quat('Y', 'Z')


def smooth(t):
    return t * t * (3 - 2 * t)


IDLE_FRAMES = 240
# Wide sideways, so the drone clears the doorway's posts; at the back it passes behind the doorway.
IDLE_RADIUS, IDLE_HEIGHT = (3.3, 2.6), 2.55


def idle_drone(f):
    theta = 2 * math.pi * f / IDLE_FRAMES
    pos = orbit_point(theta, IDLE_RADIUS, IDLE_HEIGHT, 0.12 * math.sin(4 * theta))
    return pos, aim(pos, LOOK_AT)


def visit(f, frames, target_pos, target_look):
    """A clip that leaves the idle start, holds at target_pos looking at target_look and comes back."""
    start_pos, start_rot = idle_drone(0)
    goal_rot = aim(target_pos, target_look)
    t = f / frames
    if t < 0.35:
        k = smooth(t / 0.35)
    elif t < 0.65:
        k = 1.0
    else:
        k = smooth((1 - t) / 0.35)
    pos = start_pos.lerp(Vector(target_pos), k)
    return pos, start_rot.slerp(goal_rot, k)


def flyby_drone(f, frames=105):
    """A low pass from the left to the right in front of the player, then back up to the orbit."""
    start_pos, _ = idle_drone(0)
    t = f / frames
    low = LOOK_AT.z - 0.35
    a = Vector((-3.0, PLAYER_AT.y - 1.7, low))
    b = Vector((3.0, PLAYER_AT.y - 1.7, low))
    if t < 0.25:
        pos = start_pos.lerp(a, smooth(t / 0.25))
    elif t < 0.75:
        pos = a.lerp(b, smooth((t - 0.25) / 0.5))
    else:
        pos = b.lerp(start_pos, smooth((t - 0.75) / 0.25))
    return pos, aim(pos, LOOK_AT)


CLIPS = {
    # name: (frames, drone(f) -> (position, rotation))
    'idle': (IDLE_FRAMES, idle_drone),
    'flyby': (105, flyby_drone),
    'top': (105, lambda f: visit(f, 105, LOOK_AT + Vector((0.0, -0.4, 3.2)), LOOK_AT)),
    'shoulder': (105, lambda f: visit(f, 105, PLAYER_AT + Vector((0.7, 0.9, 2.05)), PLAYER_AT + Vector((0, -3, 1.2)))),
}


def poses(clip, f, rig):
    """Every animated object's (location, rotation quaternion, scale) at frame f of a clip."""
    frames, drone_at = CLIPS[clip]
    out = {}
    pos, rot = drone_at(f)
    out[rig['drone']] = (pos, rot, Vector((1, 1, 1)))
    blink = 1.0 if (f // 15) % 2 == 0 else 0.0
    out[rig['lamp']] = (rig['lamp'].location.copy(), Quaternion(), Vector((blink,) * 3))
    spin = 2 * math.pi * 12 * f / frames  # whole turns, so a loop has no seam
    for i, hub in enumerate(rig['rotors']):
        sign = 1 if i in (0, 3) else -1
        out[hub] = (hub.location.copy(), Quaternion((0, 0, 1), sign * spin), Vector((1, 1, 1)))
    # The player walks in place; their head follows the drone a little.
    walk = 2 * math.pi * f * max(1, round(frames / 30)) / frames  # whole steps per clip
    swing = math.radians(32) * math.sin(walk)
    for arm, leg, sign in zip(rig['arms'], rig['legs'], (1, -1)):
        out[arm] = (arm.location.copy(), Quaternion((1, 0, 0), sign * swing * 0.8), Vector((1, 1, 1)))
        out[leg] = (leg.location.copy(), Quaternion((1, 0, 0), -sign * swing), Vector((1, 1, 1)))
    to_drone = pos - (PLAYER_AT + Vector((0, 0, 1.6)))
    yaw = math.atan2(to_drone.x, -to_drone.y)
    yaw = max(-0.7, min(0.7, yaw)) * 0.6
    pitch = max(-0.35, min(0.35, math.atan2(to_drone.z, to_drone.xy.length))) * 0.5
    head_rot = Quaternion((0, 0, 1), yaw) @ Quaternion((1, 0, 0), -pitch)
    out[rig['head']] = (rig['head'].location.copy(), head_rot, Vector((1, 1, 1)))
    bob = 0.03 * abs(math.sin(walk))
    out[rig['player']] = (PLAYER_AT + Vector((0, 0, bob)), Quaternion(), Vector((1, 1, 1)))
    # The clapperboard claps once at the start of the loop and of every clip.
    clap = 0.0 if f > 12 else math.radians(30) * math.sin(math.pi * f / 12)
    out[rig['stick']] = (rig['stick'].location.copy(), Quaternion((0, 1, 0), -clap), Vector((1, 1, 1)))
    return out


def key_clips(rig, clips):
    """One slotted Action per clip on an NLA track named as the clip (NLA_TRACKS export), every
    animated object keyed at every frame with linear interpolation."""
    animated = [rig['drone'], rig['lamp'], rig['head'], rig['player'], rig['stick'], *rig['rotors'], *rig['arms'], *rig['legs']]
    for ob in animated:
        ob.rotation_mode = 'QUATERNION'
    for clip in clips:
        frames = CLIPS[clip][0]
        samples = {ob: [] for ob in animated}
        prev = {}
        for f in range(frames + 1):
            for ob, (loc, rot, scl) in poses(clip, f % frames if clip == 'idle' else f, rig).items():
                if ob in prev and prev[ob].dot(rot) < 0:
                    rot = -rot  # the shorter way between two keys
                prev[ob] = rot
                samples[ob].append((f, loc, rot, scl))
        act = bpy.data.actions.new(f'letscam_{clip}')
        act.use_fake_user = True
        act.use_frame_range = True
        act.frame_range = (0, frames)
        act.use_cyclic = clip == 'idle'
        strip = act.layers.new('keys').strips.new(type='KEYFRAME')
        for ob in animated:
            slot = act.slots.new(id_type='OBJECT', name=ob.name)
            bag = strip.channelbag(slot, ensure=True)
            for path, size, pick in (('location', 3, 1), ('rotation_quaternion', 4, 2), ('scale', 3, 3)):
                for i in range(size):
                    fc = bag.fcurves.new(path, index=i)
                    fc.keyframe_points.add(len(samples[ob]))
                    for kp, s in zip(fc.keyframe_points, samples[ob]):
                        kp.co = (s[0], s[pick][i])
                        kp.interpolation = 'CONSTANT' if ob is rig['lamp'] and path == 'scale' else 'LINEAR'
                    fc.update()
            ad = ob.animation_data_create()
            track = ad.nla_tracks.new()
            track.name = clip
            st = track.strips.new(clip, 0, act)
            st.action_slot = slot
            st.extrapolation = 'HOLD'


def key_drone_machine(drone, rotors):
    """The machine model is still: the site spins drill_* and pulses lit_* itself."""
    drone.rotation_mode = 'QUATERNION'
    drone.rotation_quaternion = Quaternion((0, 0, 1), math.pi)  # the lens faces +Z in glTF


# ---------------------------------------------------------------------------- export

def export(objects, path):
    for ob in bpy.context.scene.objects:
        ob.select_set(False)
    for ob in objects:
        ob.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=path, export_format='GLB', use_selection=True, export_apply=False, export_yup=True,
        export_texcoords=True, export_normals=True, export_materials='EXPORT', export_image_format='AUTO',
        export_animations=True, export_animation_mode='NLA_TRACKS', export_force_sampling=True,
        export_frame_step=1, export_optimize_animation_size=True, export_optimize_animation_keep_anim_object=True,
        export_skins=False, export_morph=False, export_cameras=False, export_lights=False, export_extras=False)
    size = os.path.getsize(path)
    assert size <= SIZE_LIMIT, f'{path}: {size} bytes, over the site limit of {SIZE_LIMIT}'
    print(f'letscam scene: {os.path.relpath(path, ROOT)} {size // 1024} KB')


def descendants(ob):
    out = [ob]
    for ch in ob.children:
        out += descendants(ch)
    return out


def preview(folder, rig, scene_roots):
    """EEVEE stills of every clip from the site's usual three-quarter view, to check by eye."""
    os.makedirs(folder, exist_ok=True)
    scene = bpy.context.scene
    scene.render.engine = 'BLENDER_EEVEE'
    scene.render.resolution_x, scene.render.resolution_y = 960, 640
    world = bpy.data.worlds.new('preview')
    world.use_nodes = True
    world.node_tree.nodes['Background'].inputs['Color'].default_value = (0.09, 0.07, 0.06, 1)
    scene.world = world
    sun = bpy.data.objects.new('sun', bpy.data.lights.new('sun', 'SUN'))
    sun.data.energy = 3.0
    sun.rotation_euler = (math.radians(50), 0, math.radians(30))
    scene.collection.objects.link(sun)
    cam = bpy.data.objects.new('cam', bpy.data.cameras.new('cam'))
    scene.collection.objects.link(cam)
    cam.location = (7.5, -9.5, 6.5)
    cam.rotation_mode = 'QUATERNION'
    cam.rotation_quaternion = (Vector((0, 0, 1.4)) - cam.location).to_track_quat('-Z', 'Y')
    cam.data.lens = 42
    scene.camera = cam
    animated = [ob for ob in scene.objects if ob.animation_data]
    for clip in CLIPS:
        for ob in animated:
            for tr in ob.animation_data.nla_tracks:
                tr.mute = tr.name != clip
        frames = CLIPS[clip][0]
        for k in range(4):
            f = int(frames * k / 4)
            scene.frame_set(f)
            scene.render.filepath = os.path.join(folder, f'{clip}_{f:03d}.png')
            bpy.ops.render.render(write_still=True)
    for ob in animated:
        for tr in ob.animation_data.nla_tracks:
            tr.mute = tr.name != 'idle'


def main():
    argv = sys.argv[sys.argv.index('--') + 1:] if '--' in sys.argv else []
    preview_dir = argv[argv.index('--preview') + 1] if '--preview' in argv else None
    bpy.ops.wm.read_factory_settings(use_empty=True)
    scene = bpy.context.scene
    scene.render.fps = FPS
    os.makedirs(OUT, exist_ok=True)

    # The drone alone, a machine for the site.
    machine, _lamp, machine_rotors = build_drone('drone_machine', rotor_prefix='drill')
    key_drone_machine(machine, machine_rotors)
    export(descendants(machine), os.path.join(OUT, 'drone.glb'))
    for ob in descendants(machine):
        bpy.data.objects.remove(ob)

    ground = build_ground()
    doorway = build_doorway()
    player, head, arms, legs = build_player()
    drone, lamp, rotors = build_drone()
    build_frame(drone)
    props, stick = build_props()
    root = empty('letscam_scene')
    for ob in (ground, doorway, player, drone, props):
        set_parent(ob, root)
    rig = {'drone': drone, 'lamp': lamp, 'rotors': rotors, 'head': head, 'arms': arms, 'legs': legs,
           'player': player, 'stick': stick}
    key_clips(rig, CLIPS)
    export(descendants(root), os.path.join(OUT, 'letscam.glb'))
    if preview_dir:
        preview(preview_dir, rig, [root])


main()
